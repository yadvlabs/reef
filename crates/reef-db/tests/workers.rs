//! Background work shared between instances: two workers over one database must never
//! double-deliver a webhook, double-sync a mirror or double-assign a job, and the webhook
//! outbox must release an event only together with the deliveries it produced. Runs on SQLite
//! by default; set REEF_TEST_DATABASE_URL to a PostgreSQL URL to run the same tests there
//! (where the claims use `FOR UPDATE SKIP LOCKED`).

use reef_core::Id;
use reef_db::Db;
use reef_db::accounts::NewUser;
use reef_db::actions::{NewJob, NewRun};
use reef_db::mirrors::MirrorPatch;
use reef_db::repos::NewRepo;
use reef_db::webhooks::{HookInput, NewDelivery};
use std::collections::HashSet;

/// The delivery tests claim from one shared table, so they run one at a time (on PostgreSQL
/// the database is shared with every other test in this binary).
static CLAIMS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn url() -> String {
    std::env::var("REEF_TEST_DATABASE_URL").unwrap_or_else(|_| {
        let dir = std::env::temp_dir().join(format!("reef-workers-{}", reef_core::token::random_hex(6)));
        format!("sqlite://{}/reef.db", dir.display())
    })
}

/// Two independent connections (pools) to the same database, as two instances would have.
async fn two(url: &str) -> (Db, Db) {
    let a = Db::connect(url).await.expect("connect");
    let b = Db::connect(url).await.expect("connect");
    (a, b)
}

/// A hook to attach deliveries to; every test uses its own so the shared Postgres database
/// needs no truncation and the tests can run in parallel.
async fn hook(db: &Db) -> Id {
    let owner = Id(reef_core::id::next().0);
    let h = HookInput {
        url: "https://example.invalid/hook".into(),
        content_type: "json".into(),
        events: vec!["*".into()],
        active: true,
        ..Default::default()
    };
    db.create_hook("repo", owner, &h).await.unwrap().id()
}

async fn user_and_repo(db: &Db, tag: &str) -> (Id, Id) {
    let suffix = reef_core::token::random_hex(4);
    let u = db
        .create_user(NewUser {
            login: &format!("w-{tag}-{suffix}"),
            email: &format!("w-{tag}-{suffix}@example.com"),
            password: None,
            name: None,
            is_admin: false,
        })
        .await
        .unwrap();
    let r = db.create_repo(u.id(), NewRepo { name: format!("r-{tag}-{suffix}"), ..Default::default() }).await.unwrap();
    (u.id(), r.id())
}

#[tokio::test]
async fn two_workers_never_claim_the_same_delivery() {
    let _serial = CLAIMS.lock().await;
    let url = url();
    let (a, b) = two(&url).await;
    let hook = hook(&a).await;
    const N: usize = 60;
    for i in 0..N {
        a.create_delivery(hook, &format!("guid-{i}"), "push", None, "{}", "{}", None, None).await.unwrap();
    }
    // Both workers claim in small batches until nothing is left; nothing is finished in the
    // meantime, so every row stays a candidate as far as the `pending` filter goes and only
    // the claim itself keeps the other worker off it.
    let worker = |db: Db| async move {
        let mut got = Vec::new();
        loop {
            let claimed = db.claim_deliveries(7, 60_000).await.unwrap();
            if claimed.is_empty() {
                break;
            }
            got.extend(claimed.into_iter().filter(|d| d.hook_id == hook.0).map(|d| d.id));
            tokio::task::yield_now().await;
        }
        got
    };
    let (ga, gb) = tokio::join!(worker(a.clone()), worker(b.clone()));
    let sa: HashSet<i64> = ga.iter().copied().collect();
    let sb: HashSet<i64> = gb.iter().copied().collect();
    assert_eq!(sa.len(), ga.len(), "worker a claimed a delivery twice");
    assert_eq!(sb.len(), gb.len(), "worker b claimed a delivery twice");
    assert!(sa.is_disjoint(&sb), "both workers claimed the same delivery: {:?}", sa.intersection(&sb).collect::<Vec<_>>());
    assert_eq!(sa.len() + sb.len(), N, "every delivery is claimed exactly once");
    for d in a.deliveries_for_hook(hook, 500).await.unwrap() {
        assert_eq!(d.attempt, 1, "one attempt per claim");
        assert!(d.lock_until.is_some());
    }
}

#[tokio::test]
async fn a_lost_lease_cannot_be_renewed() {
    let _serial = CLAIMS.lock().await;
    let url = url();
    let (a, b) = two(&url).await;
    let hook = hook(&a).await;
    let d = a.create_delivery(hook, "guid-lease", "push", None, "{}", "{}", None, None).await.unwrap();
    // Claim batches until `d` is among them (a shared database may hold older pending rows).
    async fn claim(db: &Db, id: i64, lock_ms: i64) -> reef_db::webhooks::Delivery {
        for _ in 0..50 {
            let batch = db.claim_deliveries(200, lock_ms).await.unwrap();
            if let Some(d) = batch.iter().find(|x| x.id == id) {
                return d.clone();
            }
            assert!(!batch.is_empty(), "delivery {id} is not claimable");
        }
        panic!("delivery {id} not found in 50 batches");
    }
    // Worker a claims with a lease that expires at once.
    let mine = claim(&a, d.id, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    // Worker b takes it over after the lease expired.
    let theirs = claim(&b, d.id, 60_000).await;
    assert_eq!(theirs.attempt, 2);
    assert_eq!(a.renew_delivery_lock(d.id(), mine.lock_until.unwrap(), 60_000).await.unwrap(), None, "a lost the claim");
    let renewed = b.renew_delivery_lock(d.id(), theirs.lock_until.unwrap(), 60_000).await.unwrap();
    assert!(renewed.is_some(), "b still holds it");
    assert!(b.renew_delivery_lock(d.id(), renewed.unwrap(), 60_000).await.unwrap().is_some(), "renewal chains");
    b.finish_delivery(d.id(), "delivered", Some(200), None, None, 1, None).await.unwrap();
    assert_eq!(b.renew_delivery_lock(d.id(), renewed.unwrap(), 60_000).await.unwrap(), None, "finished rows are not renewable");
}

/// Record a repo event with a given id and its outbox entry on `tx` (what `record_event`
/// does, with the id chosen by the test).
async fn insert_event(tx: &mut sqlx::Transaction<'static, sqlx::Any>, id: i64, repo: Id) {
    let t = reef_core::time::now_ms();
    sqlx::query("INSERT INTO repo_events (id, repo_id, actor_id, kind, payload, created_at) VALUES ($1, $2, NULL, 'push', '{}', $3)")
        .bind(id)
        .bind(repo.0)
        .bind(t)
        .execute(&mut **tx)
        .await
        .unwrap();
    reef_db::webhooks::enqueue_outbox(&mut **tx, reef_db::webhooks::OUTBOX_REPO_EVENTS, id, t).await.unwrap();
}

/// The webhook outbox: entries are claimed by one worker at a time, removed only together
/// with the deliveries recorded for them, and a claim lost to a sibling commits nothing.
#[tokio::test]
async fn the_outbox_commits_entries_only_with_their_deliveries() {
    let _serial = CLAIMS.lock().await;
    let url = url();
    let (a, b) = two(&url).await;
    let hook = hook(&a).await;
    let (_, repo) = user_and_repo(&a, "outbox").await;
    a.purge_outbox_before(reef_core::time::now_ms() + 1).await.unwrap();
    let e1 = a.record_event(repo, None, "push", &serde_json::json!({})).await.unwrap().0;
    let e2 = a.record_event(repo, None, "push", &serde_json::json!({})).await.unwrap().0;
    let ours = |c: &reef_db::webhooks::OutboxClaim| -> Vec<i64> {
        c.entries.iter().filter(|e| e.event_id == e1 || e.event_id == e2).map(|e| e.event_id).collect()
    };
    let make = |i: usize| NewDelivery {
        hook,
        guid: format!("outbox-{i}"),
        event: "push".into(),
        action: None,
        payload: "{}".into(),
        request_headers: "{}".into(),
        repo: None,
        redelivery_of: None,
    };
    let ca = a.claim_outbox(1000, 60_000).await.unwrap();
    assert_eq!(ours(&ca), vec![e1, e2], "oldest first");
    let cb = b.claim_outbox(1000, 60_000).await.unwrap();
    assert!(ours(&cb).is_empty(), "claimed entries are not handed to a sibling");
    b.release_outbox(&cb.token).await.unwrap();
    let built: Vec<(i64, Vec<NewDelivery>)> =
        ca.entries.iter().filter(|e| e.event_id == e1 || e.event_id == e2).enumerate().map(|(i, e)| (e.id, vec![make(i)])).collect();
    // A stale claim (a sibling's token) commits nothing.
    assert_eq!(b.commit_outbox("not-the-claim", &built).await.unwrap(), 0);
    assert!(b.deliveries_for_hook(hook, 100).await.unwrap().is_empty());
    assert_eq!(a.commit_outbox(&ca.token, &built).await.unwrap(), 2);
    assert_eq!(a.deliveries_for_hook(hook, 100).await.unwrap().len(), 2);
    assert_eq!(a.commit_outbox(&ca.token, &built).await.unwrap(), 0, "exactly once");
    assert_eq!(a.deliveries_for_hook(hook, 100).await.unwrap().len(), 2);
    a.release_outbox(&ca.token).await.unwrap();
    // A dead holder's lease expires and the entry is claimable again; the old holder's commit
    // then records nothing.
    let e3 = a.record_event(repo, None, "push", &serde_json::json!({})).await.unwrap().0;
    let dead = a.claim_outbox(1000, 1).await.unwrap();
    let dead_entry = dead.entries.iter().find(|e| e.event_id == e3).expect("claimed").id;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let taken = b.claim_outbox(1000, 60_000).await.unwrap();
    assert!(taken.entries.iter().any(|e| e.event_id == e3), "expired leases are free");
    assert_eq!(a.commit_outbox(&dead.token, &[(dead_entry, vec![make(9)])]).await.unwrap(), 0);
    assert_eq!(b.commit_outbox(&taken.token, &[(dead_entry, vec![make(10)])]).await.unwrap(), 1);
    b.release_outbox(&taken.token).await.unwrap();
    assert_eq!(a.deliveries_for_hook(hook, 100).await.unwrap().len(), 3);
}

/// Ids are minted before the commit, so an event with a smaller id can commit after one with
/// a larger id was already polled. The outbox still hands it out. On PostgreSQL the two
/// transactions really overlap; on SQLite (one writer) they run one after the other.
#[tokio::test]
async fn an_event_committed_out_of_id_order_reaches_the_outbox() {
    let _serial = CLAIMS.lock().await;
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "ooo").await;
    let (early, late) = (reef_core::id::next().0, reef_core::id::next().0);
    assert!(early < late);
    let ids = |c: &reef_db::webhooks::OutboxClaim| -> Vec<i64> { c.entries.iter().map(|e| e.event_id).collect() };
    let pg = a.backend() == reef_db::Backend::Postgres;
    let mut slow = a.pool().begin().await.unwrap();
    if pg {
        // Instance A minted `early` and is still in its transaction ...
        insert_event(&mut slow, early, repo).await;
    }
    // ... while instance B records `late` and commits.
    let mut fast = b.pool().begin().await.unwrap();
    insert_event(&mut fast, late, repo).await;
    fast.commit().await.unwrap();
    let first = b.claim_outbox(1000, 60_000).await.unwrap();
    assert!(ids(&first).contains(&late));
    assert!(!ids(&first).contains(&early), "not committed yet");
    let done: Vec<(i64, Vec<NewDelivery>)> = first.entries.iter().filter(|e| e.event_id == late).map(|e| (e.id, Vec::new())).collect();
    assert_eq!(b.commit_outbox(&first.token, &done).await.unwrap(), 1);
    b.release_outbox(&first.token).await.unwrap();
    if !pg {
        insert_event(&mut slow, early, repo).await;
    }
    slow.commit().await.unwrap();
    let second = b.claim_outbox(1000, 60_000).await.unwrap();
    assert!(ids(&second).contains(&early), "the late commit with the smaller id is not skipped");
    b.release_outbox(&second.token).await.unwrap();
}

#[tokio::test]
async fn two_workers_never_sync_the_same_mirror() {
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "mirror").await;
    a.set_mirror_config(
        repo,
        MirrorPatch { pull_url: "https://example.invalid/src.git", username: None, password: None, interval_minutes: 60 },
    )
    .await
    .unwrap();
    for round in 0..20 {
        let (wa, wb) = tokio::join!(a.claim_mirror(repo, 60_000), b.claim_mirror(repo, 60_000));
        let (wa, wb) = (wa.unwrap(), wb.unwrap());
        assert!(wa.is_some() ^ wb.is_some(), "round {round}: exactly one worker must win (a={wa:?}, b={wb:?})");
        // While the lease is held the mirror is neither due nor claimable.
        assert!(!a.due_mirrors(10).await.unwrap().iter().any(|m| m.repo_id == repo.0), "a leased mirror is not due");
        assert!(b.claim_mirror(repo, 60_000).await.unwrap().is_none());
        let (winner, loser, held) = if let Some(h) = wa { (&a, &b, h) } else { (&b, &a, wb.unwrap()) };
        // Only the holder can renew or release: a stale lease value is refused and changes nothing.
        assert_eq!(loser.renew_mirror_lease(repo, held - 1, 120_000).await.unwrap(), None);
        assert!(!loser.mirror_synced(repo, held - 1, Some("not mine")).await.unwrap());
        let m = a.mirror_config(repo).await.unwrap().unwrap();
        assert_eq!((m.sync_lock_until, m.last_error.as_deref()), (Some(held), None), "still held, nothing recorded");
        let held = winner.renew_mirror_lease(repo, held, 120_000).await.unwrap().expect("the holder renews");
        assert!(winner.mirror_synced(repo, held, None).await.unwrap());
        let m = a.mirror_config(repo).await.unwrap().unwrap();
        assert_eq!(m.sync_lock_until, None, "the lease is released with the outcome");
        assert!(a.claim_mirror(repo, 60_000).await.unwrap().is_none(), "not due again until the interval passed");
        a.mirror_sync_now(repo).await.unwrap();
    }
    // An expired lease is taken over; the previous holder can then neither renew nor release it.
    let stale = a.claim_mirror(repo, 1).await.unwrap().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let fresh = b.claim_mirror(repo, 60_000).await.unwrap().expect("expired leases are free");
    assert_eq!(a.renew_mirror_lease(repo, stale, 60_000).await.unwrap(), None);
    assert!(!a.mirror_synced(repo, stale, Some("late")).await.unwrap());
    assert!(b.mirror_synced(repo, fresh, None).await.unwrap());
    // Push mirrors carry the same lease.
    let pm = a.create_push_mirror(repo, "https://example.invalid/dst.git", None, None).await.unwrap();
    let (pa, pb) = tokio::join!(a.claim_push_mirror(Id(pm.id), 60_000), b.claim_push_mirror(Id(pm.id), 60_000));
    let (pa, pb) = (pa.unwrap(), pb.unwrap());
    assert!(pa.is_some() ^ pb.is_some());
    let claim = pa.or(pb).unwrap();
    let held = claim.lease;
    assert!(!a.pending_push_mirrors(0, 10).await.unwrap().iter().any(|m| m.id == pm.id), "a leased push mirror is not pending work");
    assert_eq!(a.renew_push_mirror_lease(Id(pm.id), held - 1, 60_000).await.unwrap(), None);
    assert!(!a.push_mirror_result(Id(pm.id), held - 1, claim.generation, None).await.unwrap(), "a stale holder records nothing");
    let held = a.renew_push_mirror_lease(Id(pm.id), held, 60_000).await.unwrap().unwrap();
    assert!(a.push_mirror_result(Id(pm.id), held, claim.generation, None).await.unwrap());
    let pm = a.push_mirror(repo, Id(pm.id)).await.unwrap();
    assert_eq!((pm.pending, pm.lock_until), (0, None));
}

/// A push marked while another push of the same mirror runs is not lost: the running push
/// records its outcome without clearing `pending`, and the mirror is due again at once.
#[tokio::test]
async fn a_push_marked_during_a_push_is_pushed_again() {
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "pushgen").await;
    let pm = a.create_push_mirror(repo, "https://example.invalid/dst.git", None, None).await.unwrap();
    let id = Id(pm.id);
    let claim = a.claim_push_mirror(id, 60_000).await.unwrap().expect("a new push mirror is pending");
    // Another push lands while this one runs: the other instance cannot claim, and marks it.
    b.push_mirrors_mark_pending(repo).await.unwrap();
    assert!(b.claim_push_mirror(id, 60_000).await.unwrap().is_none(), "the lease is held");
    assert_eq!(a.push_mirror(repo, id).await.unwrap().pending, 1, "a lost claim leaves pending set");
    assert!(a.push_mirror_result(id, claim.lease, claim.generation, None).await.unwrap());
    let m = a.push_mirror(repo, id).await.unwrap();
    assert_eq!((m.pending, m.lock_until, m.last_error.as_deref()), (1, None, None), "the newer mark survives the older push");
    assert!(a.pending_push_mirrors(300_000, 100).await.unwrap().iter().any(|x| x.id == pm.id), "and is due now, not after the retry pause");
    let again = b.claim_push_mirror(id, 60_000).await.unwrap().expect("pushed again");
    assert!(again.generation > claim.generation);
    assert!(b.push_mirror_result(id, again.lease, again.generation, None).await.unwrap());
    assert_eq!(a.push_mirror(repo, id).await.unwrap().pending, 0, "nothing newer: done");
    // A failed push keeps pending and waits for the retry pause.
    a.push_mirror_mark_pending(repo, id).await.unwrap();
    let c = a.claim_push_mirror(id, 60_000).await.unwrap().unwrap();
    assert!(a.push_mirror_result(id, c.lease, c.generation, Some("remote said no")).await.unwrap());
    assert_eq!(a.push_mirror(repo, id).await.unwrap().pending, 1);
    assert!(!a.pending_push_mirrors(300_000, 100).await.unwrap().iter().any(|x| x.id == pm.id), "failures retry after the pause");
}

/// A "sync now" while another instance syncs is remembered: the holder's outcome leaves the
/// mirror due immediately, the next claim clears the request, and the schedule resumes.
#[tokio::test]
async fn a_sync_requested_during_a_sync_runs_after_it() {
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "syncreq").await;
    a.set_mirror_config(
        repo,
        MirrorPatch { pull_url: "https://example.invalid/src.git", username: None, password: None, interval_minutes: 60 },
    )
    .await
    .unwrap();
    let held = a.claim_mirror(repo, 60_000).await.unwrap().expect("a new mirror is due");
    b.mirror_sync_now(repo).await.unwrap();
    assert!(b.claim_mirror(repo, 60_000).await.unwrap().is_none(), "the lease is held");
    assert_eq!(a.mirror_config(repo).await.unwrap().unwrap().sync_requested, 1);
    assert!(a.mirror_synced(repo, held, None).await.unwrap());
    let m = a.mirror_config(repo).await.unwrap().unwrap();
    assert_eq!((m.next_sync_at, m.sync_lock_until), (Some(0), None), "the request keeps it due");
    let again = a.claim_mirror(repo, 60_000).await.unwrap().expect("synced again");
    assert_eq!(a.mirror_config(repo).await.unwrap().unwrap().sync_requested, 0, "the new sync honours the request");
    assert!(a.mirror_synced(repo, again, None).await.unwrap());
    let m = a.mirror_config(repo).await.unwrap().unwrap();
    assert!(m.next_sync_at.unwrap() > reef_core::time::now_ms() + 50 * 60_000, "back on the interval");
    assert!(b.claim_mirror(repo, 60_000).await.unwrap().is_none());
}

/// Changing a mirror's source while another instance syncs it is remembered like a "sync
/// now": the holder (which fetched from the old source) leaves it due immediately.
#[tokio::test]
async fn a_source_change_during_a_sync_runs_again_after_it() {
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "syncchange").await;
    let patch = |u: &'static str| MirrorPatch { pull_url: u, username: None, password: None, interval_minutes: 60 };
    a.set_mirror_config(repo, patch("https://example.invalid/old.git")).await.unwrap();
    let held = a.claim_mirror(repo, 60_000).await.unwrap().expect("a new mirror is due");
    assert_eq!(a.mirror_config(repo).await.unwrap().unwrap().sync_requested, 0);
    b.set_mirror_config(repo, patch("https://example.invalid/new.git")).await.unwrap();
    assert!(b.claim_mirror(repo, 60_000).await.unwrap().is_none(), "the lease is held");
    assert_eq!(a.mirror_config(repo).await.unwrap().unwrap().sync_requested, 1, "the change is remembered");
    assert!(a.mirror_synced(repo, held, None).await.unwrap());
    let m = a.mirror_config(repo).await.unwrap().unwrap();
    assert_eq!(m.next_sync_at, Some(0), "not rescheduled on the interval: due now");
    assert!(a.claim_mirror(repo, 60_000).await.unwrap().is_some(), "synced again, from the new source");
}

#[tokio::test]
async fn two_runners_never_get_the_same_job() {
    let url = url();
    let (a, b) = two(&url).await;
    let (_, repo) = user_and_repo(&a, "jobs").await;
    let wf = a.upsert_workflow(repo, ".github/workflows/ci.yml", "ci").await.unwrap();
    let run = a
        .create_run(NewRun {
            repo,
            workflow: wf.id(),
            name: "ci",
            display_title: "ci",
            event: "push",
            head_sha: "0000000000000000000000000000000000000001",
            head_branch: Some("main"),
            git_ref: "refs/heads/main",
            path: ".github/workflows/ci.yml",
            actor: None,
            event_payload: "{}",
            workflow_yaml: "on: push\njobs: {}\n",
            pull_number: None,
            inputs: None,
            error: None,
        })
        .await
        .unwrap();
    let labels = vec!["self-hosted".to_string()];
    let mk_runner = |db: &Db, name: &str| {
        let db = db.clone();
        let name = name.to_string();
        let labels = labels.clone();
        async move {
            db.create_runner("repo", repo, &name, None, None, &labels, false, false, &reef_db::actions::uuid_v4(), "{}", "{}")
                .await
                .unwrap()
        }
    };
    let (ra, rb) = (mk_runner(&a, "runner-a").await, mk_runner(&b, "runner-b").await);
    let mut wins = (0, 0);
    for i in 0..12 {
        let job = a
            .create_job(
                NewJob {
                    run: run.id(),
                    repo,
                    job_key: &format!("j{i}"),
                    name: "job",
                    matrix: None,
                    needs: &[],
                    labels: &labels,
                    status: "queued",
                    order_index: i,
                },
                1,
            )
            .await
            .unwrap();
        // Request ids are unique across the table (a shared database keeps earlier runs' rows).
        let (req_a, req_b) = (reef_core::id::next().0 & 0x7fff_ffff_ffff, reef_core::id::next().0 & 0x7fff_ffff_ffff);
        let (ca, cb) = tokio::join!(
            a.assign_job(job.id(), &ra, req_a, "tok-a", 60_000, "{}"),
            b.assign_job(job.id(), &rb, req_b, "tok-b", 60_000, "{}")
        );
        let (ca, cb) = (ca.unwrap(), cb.unwrap());
        assert!(ca ^ cb, "job {i}: exactly one runner must get it (a={ca}, b={cb})");
        if ca {
            wins.0 += 1
        } else {
            wins.1 += 1
        }
        let j = a.job_by_id(job.id()).await.unwrap();
        assert_eq!(j.status, "in_progress");
        assert_eq!(j.runner_id, Some(if ca { ra.id } else { rb.id }));
        assert_eq!(j.request_id, Some(if ca { req_a } else { req_b }));
    }
    assert_eq!(wins.0 + wins.1, 12);
    let counts = a.count_jobs_by_state().await.unwrap();
    assert!(counts.running >= 12, "{counts:?}");
}

/// The retention walk over finished runs uses the `workflow_runs_finished` expression index
/// (migration 0004) on both dialects instead of scanning every run.
#[tokio::test]
async fn the_retention_walk_uses_the_finish_index() {
    let db = Db::connect(&url()).await.unwrap();
    let sql = reef_db::actions::runs_completed_before_sql(100);
    let plan: String = if db.backend() == reef_db::Backend::Postgres {
        let mut tx = db.pool().begin().await.unwrap();
        sqlx::query("SET LOCAL enable_seqscan = off").execute(&mut *tx).await.unwrap();
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN {sql}")))
            .bind(1_i64)
            .bind(i64::MIN)
            .bind(i64::MIN)
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        rows.iter().map(|r| sqlx::Row::get::<String, _>(r, 0)).collect::<Vec<_>>().join("\n")
    } else {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .bind(1_i64)
            .bind(i64::MIN)
            .bind(i64::MIN)
            .fetch_all(db.pool())
            .await
            .unwrap();
        rows.iter().map(|r| sqlx::Row::get::<String, _>(r, "detail")).collect::<Vec<_>>().join("\n")
    };
    assert!(plan.contains("workflow_runs_finished"), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "the index gives the order, no sort: {plan}");
}
