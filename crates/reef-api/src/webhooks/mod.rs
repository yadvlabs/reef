//! Webhooks: repository and organization hooks, GitHub-shaped payloads, signed deliveries
//! with a retrying background dispatcher, and the delivery log.
//!
//! Events are not emitted at the call sites that record them; instead the database layer
//! queues every row it inserts into `repo_events`, `issue_events` and `issue_comments` in the
//! webhook outbox, in the same transaction, so every event recorded anywhere (REST, GraphQL,
//! git push, Actions) reaches hooks without touching those code paths. [`tick`] runs the
//! poller and the dispatcher every 30 s; [`kick`] runs one pass immediately for callers that
//! want lower latency.
//!
//! Several instances may run the poller and the dispatcher at once:
//! - The poller drains the outbox. It claims a batch of entries under a lease, builds their
//!   deliveries, then records them and deletes the entries in one transaction
//!   (`Db::commit_outbox`), each entry only while the claim still holds it. An event is
//!   therefore turned into deliveries exactly once, and an event whose transaction commits
//!   late, after events with larger ids were already delivered (ids are minted before the
//!   commit, on every instance), is still picked up: there is no id cursor to have moved
//!   past it.
//! - The dispatcher claims delivery rows under a lease (`FOR UPDATE SKIP LOCKED` on
//!   PostgreSQL, a guarded update on SQLite) and renews the lease before each post, so a slow
//!   endpoint earlier in a batch cannot let a sibling re-claim what is still being worked on.

pub mod payloads;
pub mod rest;

use crate::AppState;
use hmac::{Hmac, Mac};
use reef_core::Id;
use reef_db::webhooks::{Delivery, NewDelivery, Webhook};
use serde_json::{Value, json};
use sha1::Sha1;
use sha2_010::Sha256;
use std::collections::BTreeMap;
use std::time::Instant;

/// Attempts after the first, with the delay before each.
const RETRY_DELAYS_MS: [i64; 3] = [60_000, 300_000, 1_800_000];
const MAX_ATTEMPTS: i64 = 1 + RETRY_DELAYS_MS.len() as i64;
const LOCK_MS: i64 = 60_000;
/// How long the poller's lease on a batch of outbox entries lasts. A build never takes near
/// this long; the lease only keeps a second instance from building the same entries at once.
const OUTBOX_LEASE_MS: i64 = 60_000;
/// Outbox entries per poller pass.
const OUTBOX_BATCH: i64 = 200;
/// While no hook exists the outbox is emptied, except for entries this recent: an event
/// recorded just after a hook was created is kept however the instances' clocks differ.
const OUTBOX_IDLE_GRACE_MS: i64 = 60_000;
const RESPONSE_BODY_LIMIT: usize = 64 * 1024;
const RETENTION_MS: i64 = 30 * 24 * 3600 * 1000;

/// Hex HMAC-SHA256 of `body` with `secret`, as sent in `X-Hub-Signature-256`.
pub fn signature_sha256(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Hex HMAC-SHA1 of `body`, the legacy `X-Hub-Signature`.
pub fn signature_sha1(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(body);
    format!("sha1={}", hex::encode(mac.finalize().into_bytes()))
}

/// Called from the server's periodic tick: pick up new events, then dispatch due deliveries.
pub async fn tick(s: &AppState) {
    kick(s).await;
    let _ = s.db.purge_deliveries_before(reef_core::time::now_ms() - RETENTION_MS).await;
}

/// One poll pass followed by one dispatch pass. Safe to call from anywhere after an event
/// has been recorded when 30 s of latency is too much.
pub async fn kick(s: &AppState) {
    if let Err(e) = poll(s).await {
        tracing::warn!(error = %e, "webhooks: polling events failed");
    }
    if let Err(e) = dispatch(s).await {
        tracing::warn!(error = %e, "webhooks: dispatch failed");
    }
}

// ----- poller -----

async fn poll(s: &AppState) -> reef_core::Result<()> {
    let started = reef_core::time::now_ms();
    if !s.db.any_active_hook().await? {
        // Nothing to deliver to: drop what piled up so enabling a hook never replays history.
        s.db.purge_outbox_before(started - OUTBOX_IDLE_GRACE_MS).await?;
        return Ok(());
    }
    // Drain in batches; a pass ends when the outbox is empty (or held by siblings).
    loop {
        let claim = s.db.claim_outbox(OUTBOX_BATCH, OUTBOX_LEASE_MS).await?;
        if claim.entries.is_empty() {
            return Ok(());
        }
        let n = claim.entries.len();
        let mut built = Vec::with_capacity(n);
        for e in &claim.entries {
            match build_entry(s, e).await {
                Ok(out) => built.push((e.id, out)),
                Err(err) => {
                    let _ = s.db.release_outbox(&claim.token).await;
                    return Err(err);
                }
            }
        }
        let committed = s.db.commit_outbox(&claim.token, &built).await?;
        if committed < n {
            tracing::debug!(lost = n - committed, "webhooks: outbox entries were re-claimed by a sibling instance");
        }
        if (n as i64) < OUTBOX_BATCH {
            return Ok(());
        }
    }
}

/// The deliveries of one outbox entry (none when the event is gone or nobody listens).
async fn build_entry(s: &AppState, e: &reef_db::webhooks::OutboxEntry) -> reef_core::Result<Vec<NewDelivery>> {
    use reef_db::webhooks::{OUTBOX_ISSUE_COMMENTS, OUTBOX_ISSUE_EVENTS, OUTBOX_REPO_EVENTS};
    let mut out = Vec::new();
    match e.source.as_str() {
        OUTBOX_REPO_EVENTS => {
            let Some(ev) = s.db.polled_repo_event(e.event_id).await? else { return Ok(out) };
            let Ok(rw) = s.db.repo_with_owner_by_id(Id(ev.repo_id)).await else { return Ok(out) };
            let hooks = hooks_for(s, &rw).await?;
            if hooks.is_empty() {
                return Ok(out);
            }
            let actor = match ev.actor_id {
                Some(a) => s.db.account_by_id(Id(a)).await.ok(),
                None => None,
            };
            let payload: Value = serde_json::from_str(&ev.payload).unwrap_or(Value::Null);
            for (name, action, body) in payloads::from_repo_event(s, &rw, actor.as_ref(), &ev.kind, &payload).await {
                out.extend(plan(s, &rw, &hooks, &name, action.as_deref(), &body));
            }
        }
        OUTBOX_ISSUE_EVENTS => {
            let Some(ev) = s.db.polled_issue_event(e.event_id).await? else { return Ok(out) };
            let Ok(rw) = s.db.repo_with_owner_by_id(Id(ev.repo_id)).await else { return Ok(out) };
            let hooks = hooks_for(s, &rw).await?;
            if hooks.is_empty() {
                return Ok(out);
            }
            let actor = match ev.actor_id {
                Some(a) => s.db.account_by_id(Id(a)).await.ok(),
                None => None,
            };
            let payload: Value = serde_json::from_str(&ev.payload).unwrap_or(Value::Null);
            for (name, action, body) in payloads::from_issue_event(s, &rw, actor.as_ref(), Id(ev.issue_id), &ev.event, &payload).await {
                out.extend(plan(s, &rw, &hooks, &name, action.as_deref(), &body));
            }
        }
        OUTBOX_ISSUE_COMMENTS => {
            let Some(c) = s.db.polled_comment(e.event_id).await? else { return Ok(out) };
            let Ok(rw) = s.db.repo_with_owner_by_id(Id(c.repo_id)).await else { return Ok(out) };
            let hooks = hooks_for(s, &rw).await?;
            if hooks.is_empty() {
                return Ok(out);
            }
            let actor = s.db.account_by_id(Id(c.author_id)).await.ok();
            if let Some(body) = payloads::issue_comment(s, &rw, actor.as_ref(), Id(c.issue_id), Id(c.id)).await {
                out.extend(plan(s, &rw, &hooks, "issue_comment", Some("created"), &body));
            }
        }
        other => tracing::warn!(source = other, "webhooks: unknown outbox source; dropping the entry"),
    }
    Ok(out)
}

async fn hooks_for(s: &AppState, rw: &reef_db::repos::RepoWithOwner) -> reef_core::Result<Vec<Webhook>> {
    let org = if rw.owner.is_org() { Some(rw.owner.id()) } else { None };
    s.db.active_hooks_for_repo(rw.repo.id(), org).await
}

/// The deliveries of `body` for every hook subscribed to `event` (recorded by `commit`).
pub fn plan(
    s: &AppState,
    rw: &reef_db::repos::RepoWithOwner,
    hooks: &[Webhook],
    event: &str,
    action: Option<&str>,
    body: &Value,
) -> Vec<NewDelivery> {
    hooks
        .iter()
        .filter(|h| h.is_active() && h.wants(event))
        .map(|h| new_delivery(s, h, event, action, body, Some(rw.repo.id()), None))
        .collect()
}

fn new_delivery(
    s: &AppState,
    hook: &Webhook,
    event: &str,
    action: Option<&str>,
    body: &Value,
    repo: Option<Id>,
    redelivery_of: Option<Id>,
) -> NewDelivery {
    let guid = reef_db::actions::uuid_v4();
    let headers = request_headers(s, hook, event, &guid, body);
    NewDelivery {
        hook: hook.id(),
        guid,
        event: event.to_string(),
        action: action.map(String::from),
        payload: body.to_string(),
        request_headers: serde_json::to_string(&headers).unwrap_or_default(),
        repo,
        redelivery_of,
    }
}

/// Queue one delivery for one hook right away (pings, tests and redeliveries).
pub async fn enqueue_one(
    s: &AppState,
    hook: &Webhook,
    event: &str,
    action: Option<&str>,
    body: &Value,
    repo: Option<Id>,
    redelivery_of: Option<Id>,
) -> reef_core::Result<Delivery> {
    let d = new_delivery(s, hook, event, action, body, repo, redelivery_of);
    s.db.create_delivery(hook.id(), &d.guid, event, action, &d.payload, &d.request_headers, repo, redelivery_of).await
}

/// The request headers of a delivery, computed once so the log shows exactly what was sent.
fn request_headers(s: &AppState, hook: &Webhook, event: &str, guid: &str, body: &Value) -> BTreeMap<String, String> {
    let raw = body.to_string();
    let (content_type, bytes) = encode_body(hook, &raw);
    let mut h = BTreeMap::new();
    h.insert("Accept".into(), "*/*".into());
    h.insert("Content-Type".into(), content_type.to_string());
    h.insert("User-Agent".into(), "GitHub-Hookshot/reef".into());
    h.insert("X-GitHub-Delivery".into(), guid.to_string());
    h.insert("X-GitHub-Event".into(), event.to_string());
    h.insert("X-GitHub-Hook-ID".into(), hook.id.to_string());
    h.insert("X-GitHub-Hook-Installation-Target-ID".into(), hook.owner_id.to_string());
    h.insert("X-GitHub-Hook-Installation-Target-Type".into(), if hook.owner_kind == "org" { "organization" } else { "repository" }.into());
    h.insert("X-Reef-Instance".into(), s.config.site_url().to_string());
    if let Some(secret) = hook.secret.as_deref().filter(|x| !x.is_empty()) {
        h.insert("X-Hub-Signature".into(), signature_sha1(secret, &bytes));
        h.insert("X-Hub-Signature-256".into(), signature_sha256(secret, &bytes));
    }
    h
}

/// The wire body for a hook: the JSON itself, or `payload=<urlencoded json>` for form hooks.
fn encode_body(hook: &Webhook, raw_json: &str) -> (&'static str, Vec<u8>) {
    if hook.content_type == "form" {
        let encoded = serde_urlencoded::to_string([("payload", raw_json)]).unwrap_or_default();
        ("application/x-www-form-urlencoded", encoded.into_bytes())
    } else {
        ("application/json", raw_json.as_bytes().to_vec())
    }
}

// ----- dispatcher -----

async fn dispatch(s: &AppState) -> reef_core::Result<()> {
    let due = s.db.claim_deliveries(50, LOCK_MS).await?;
    for d in due {
        let Some(d) = renew(s, d).await else { continue };
        let Ok(hook) = s.db.hook_by_id(Id(d.hook_id)).await else {
            let _ = s.db.finish_delivery(d.id(), "failed", None, None, Some("hook no longer exists"), 0, None).await;
            crate::metrics::registry().webhook_deliveries.with_label_values(&["failed"]).inc();
            continue;
        };
        deliver(s, &hook, &d).await;
    }
    Ok(())
}

/// Extend the lease on a claimed delivery right before posting it. A batch is posted one
/// delivery at a time and each post may take up to the HTTP timeout, so the lease taken at
/// claim time can run out before the last rows are reached; renewing per row keeps a sibling
/// from re-claiming what is still ours. `None` when the lease was lost (the sibling has it).
async fn renew(s: &AppState, d: Delivery) -> Option<Delivery> {
    let held = d.lock_until?;
    match s.db.renew_delivery_lock(d.id(), held, LOCK_MS).await {
        Ok(Some(until)) => Some(Delivery { lock_until: Some(until), ..d }),
        Ok(None) => {
            tracing::debug!(delivery = d.id, "webhooks: lease lost before posting; another instance has it");
            None
        }
        Err(e) => {
            tracing::warn!(delivery = d.id, error = %e, "webhooks: renewing the lease failed");
            None
        }
    }
}

async fn deliver(s: &AppState, hook: &Webhook, d: &Delivery) {
    let headers: BTreeMap<String, String> = serde_json::from_str(&d.request_headers).unwrap_or_default();
    let (_, bytes) = encode_body(hook, &d.payload);
    let client: Result<reqwest::Client, std::io::Error> = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .danger_accept_invalid_certs(hook.insecure_ssl != 0)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(std::io::Error::other);
    let started = Instant::now();
    // Re-check the target at delivery time: DNS may have changed since the hook was saved.
    let client = match crate::outbound::check_url(s, &hook.url).await {
        Ok(()) => client,
        Err(r) => Err::<reqwest::Client, _>(std::io::Error::other(r.to_string())),
    };
    let result = match client {
        Ok(c) => {
            let mut req = c.post(&hook.url);
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req.body(bytes).send().await.map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    let duration = started.elapsed().as_millis() as i64;
    let now = reef_core::time::now_ms();
    match result {
        Ok(resp) => {
            let status = resp.status().as_u16() as i64;
            let rh: BTreeMap<String, String> =
                resp.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
            let body = resp.text().await.unwrap_or_default();
            let body: String = body.chars().take(RESPONSE_BODY_LIMIT).collect();
            let retry = status >= 500 && d.attempt < MAX_ATTEMPTS;
            let outcome = if (200..300).contains(&status) {
                "delivered"
            } else if retry {
                "pending"
            } else {
                "failed"
            };
            let next = if retry { Some(now + RETRY_DELAYS_MS[(d.attempt - 1).clamp(0, 2) as usize]) } else { None };
            let _ =
                s.db.finish_delivery(
                    d.id(),
                    outcome,
                    Some(status),
                    Some(&serde_json::to_string(&rh).unwrap_or_default()),
                    Some(&body),
                    duration,
                    next,
                )
                .await;
            let _ = s.db.set_hook_last(hook.id(), Some(status), now).await;
            count(if outcome == "pending" { "retried" } else { outcome });
        }
        Err(e) => {
            let retry = d.attempt < MAX_ATTEMPTS;
            let next = if retry { Some(now + RETRY_DELAYS_MS[(d.attempt - 1).clamp(0, 2) as usize]) } else { None };
            let _ =
                s.db.finish_delivery(
                    d.id(),
                    if retry { "pending" } else { "failed" },
                    None,
                    None,
                    Some(&format!("delivery error: {e}")),
                    duration,
                    next,
                )
                .await;
            let _ = s.db.set_hook_last(hook.id(), None, now).await;
            count(if retry { "retried" } else { "failed" });
        }
    }
}

/// `reef_webhook_deliveries_total{result}`: delivered, retried (5xx or network error with
/// attempts left) or failed.
fn count(result: &str) {
    crate::metrics::registry().webhook_deliveries.with_label_values(&[result]).inc();
}

/// Deliver one delivery right now (ping, test, redeliver), bypassing the tick. Only the
/// requested row is claimed, like the dispatcher would (so a concurrent tick cannot
/// double-post it); the rest of the backlog is left to the dispatcher. The post runs detached
/// (`timeout::detached`): a request that times out or hangs up does not abandon it mid-post.
pub async fn deliver_now(s: &AppState, hook: &Webhook, delivery_id: Id) {
    let (s, hook) = (s.clone(), hook.clone());
    crate::timeout::detached(async move {
        match s.db.claim_delivery(delivery_id, LOCK_MS).await {
            Ok(Some(d)) => deliver(&s, &hook, &d).await,
            Ok(None) => tracing::debug!(delivery = delivery_id.0, "webhooks: delivery already taken; the dispatcher has it"),
            Err(e) => {
                tracing::warn!(delivery = delivery_id.0, error = %e, "webhooks: claiming a delivery failed; the dispatcher will retry")
            }
        }
    })
    .await
}

/// The `ping` payload GitHub sends when a hook is created or pinged.
pub fn ping_payload(
    s: &AppState,
    hook: &Webhook,
    hook_json: Value,
    repository: Option<Value>,
    organization: Option<Value>,
    sender: Value,
) -> Value {
    let mut v = json!({
        "zen": "Keep it logically awesome.",
        "hook_id": hook.id,
        "hook": hook_json,
        "sender": sender,
    });
    if let Some(r) = repository {
        v["repository"] = r;
    }
    if let Some(o) = organization {
        v["organization"] = o;
    }
    let _ = s;
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_db::repos::NewRepo;
    use reef_db::webhooks::HookInput;

    fn hook_input_clone(h: &Webhook) -> HookInput {
        HookInput {
            url: h.url.clone(),
            content_type: h.content_type.clone(),
            secret: h.secret.clone(),
            events: h.events(),
            active: h.is_active(),
            insecure_ssl: h.insecure_ssl != 0,
        }
    }

    #[tokio::test]
    async fn the_poller_turns_outbox_entries_into_deliveries_exactly_once() {
        let s = crate::testing::state("webhooks", |_| {}).await;
        let (u, _) = crate::testing::user_with_token(&s, "hooked").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        // With no hook, what piled up is dropped (older than the grace period): enabling a hook
        // never replays history.
        s.db.record_event(repo.id(), Some(u.id()), "delete", &json!({"ref": "refs/heads/old"})).await.unwrap();
        assert_eq!(s.db.count_outbox().await.unwrap(), 1, "recorded with its outbox entry");
        s.db.purge_outbox_before(reef_core::time::now_ms() + 1).await.unwrap();
        poll(&s).await.unwrap();
        assert_eq!(s.db.count_outbox().await.unwrap(), 0);
        // A `delete` repo event fans out to GitHub's `push` and `delete` hook events; this hook
        // subscribes to `delete` only, so the subscription filter is exercised too.
        let hook = HookInput {
            url: "https://example.invalid/h".into(),
            content_type: "json".into(),
            events: vec!["delete".into()],
            active: true,
            ..Default::default()
        };
        let hook = s.db.create_hook("repo", repo.id(), &hook).await.unwrap();
        let everything = HookInput { events: vec!["*".into()], ..hook_input_clone(&hook) };
        let all = s.db.create_hook("repo", repo.id(), &everything).await.unwrap();
        s.db.record_event(repo.id(), Some(u.id()), "delete", &json!({"ref": "refs/heads/topic"})).await.unwrap();
        s.db.record_event(repo.id(), Some(u.id()), "delete", &json!({"ref": "refs/tags/v0"})).await.unwrap();
        // While another instance holds the entries, this one leaves them alone.
        let sibling = s.db.claim_outbox(100, 60_000).await.unwrap();
        assert_eq!(sibling.entries.len(), 2);
        poll(&s).await.unwrap();
        assert!(s.db.deliveries_for_hook(hook.id(), 10).await.unwrap().is_empty(), "another instance is building these");
        s.db.release_outbox(&sibling.token).await.unwrap();
        poll(&s).await.unwrap();
        let ds = s.db.deliveries_for_hook(hook.id(), 10).await.unwrap();
        assert_eq!(ds.len(), 2, "one delivery per event for the subscribed hook");
        assert!(ds.iter().all(|d| d.event == "delete" && d.status == "pending" && d.repo_id == Some(repo.id)));
        let every = s.db.deliveries_for_hook(all.id(), 10).await.unwrap();
        assert_eq!(every.len(), 4, "a hook on everything also gets the push events");
        assert_eq!(every.iter().filter(|d| d.event == "push").count(), 2);
        let headers: BTreeMap<String, String> = serde_json::from_str(&ds[0].request_headers).unwrap();
        assert_eq!(headers["X-GitHub-Event"], "delete");
        assert_eq!(s.db.count_outbox().await.unwrap(), 0, "committed entries are gone");
        // Nothing is produced twice, and the sibling's stale claim commits nothing.
        poll(&s).await.unwrap();
        assert_eq!(s.db.deliveries_for_hook(hook.id(), 10).await.unwrap().len(), 2);
        assert_eq!(s.db.deliveries_for_hook(all.id(), 10).await.unwrap().len(), 4);
        let stale: Vec<_> = sibling.entries.iter().map(|e| (e.id, Vec::new())).collect();
        assert_eq!(s.db.commit_outbox(&sibling.token, &stale).await.unwrap(), 0);
        // Issue comments go through the outbox too.
        let issue = s.db.create_issue(repo.id(), &u, reef_db::issues::NewIssue { title: "t".into(), ..Default::default() }).await.unwrap();
        s.db.create_comment(issue.issue.id(), &u, "hello").await.unwrap();
        poll(&s).await.unwrap();
        let every = s.db.deliveries_for_hook(all.id(), 20).await.unwrap();
        assert_eq!(
            every.iter().filter(|d| d.event == "issue_comment").count(),
            1,
            "{:?}",
            every.iter().map(|d| &d.event).collect::<Vec<_>>()
        );
        // The dispatcher's lease renewal: a claimed row is renewed, a lost one is skipped.
        let claimed = s.db.claim_deliveries(50, LOCK_MS).await.unwrap();
        assert!(claimed.len() >= 6);
        let d = renew(&s, claimed[0].clone()).await.expect("held lease renews");
        assert!(d.lock_until.unwrap() >= claimed[0].lock_until.unwrap());
        let stale = Delivery { lock_until: Some(1), ..claimed[1].clone() };
        assert!(renew(&s, stale).await.is_none(), "a lease we do not hold is not renewed");
    }

    /// Ids are minted before the commit, on every instance, so an event can commit after one
    /// with a larger id was already delivered. The outbox has no id cursor to skip it.
    #[tokio::test]
    async fn an_event_committed_out_of_id_order_is_still_delivered() {
        let s = crate::testing::state("webhooks-ooo", |_| {}).await;
        let (u, _) = crate::testing::user_with_token(&s, "ooo").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        let hook = HookInput {
            url: "https://example.invalid/h".into(),
            content_type: "json".into(),
            events: vec!["delete".into()],
            active: true,
            ..Default::default()
        };
        let hook = s.db.create_hook("repo", repo.id(), &hook).await.unwrap();
        // Two instances mint ids for two events; the one with the larger id commits first.
        let (early, late) = (reef_core::id::next().0, reef_core::id::next().0);
        assert!(early < late);
        let insert = |id: i64, r#ref: &'static str| {
            let s = s.clone();
            async move {
                let mut tx = s.db.pool().begin().await.unwrap();
                reef_db::sqlx::query(
                    "INSERT INTO repo_events (id, repo_id, actor_id, kind, payload, created_at) VALUES ($1, $2, NULL, 'delete', $3, $4)",
                )
                .bind(id)
                .bind(repo.id)
                .bind(json!({"ref": r#ref}).to_string())
                .bind(reef_core::time::now_ms())
                .execute(&mut *tx)
                .await
                .unwrap();
                reef_db::webhooks::enqueue_outbox(&mut *tx, reef_db::webhooks::OUTBOX_REPO_EVENTS, id, reef_core::time::now_ms())
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
            }
        };
        insert(late, "refs/heads/late-id").await;
        poll(&s).await.unwrap();
        assert_eq!(s.db.deliveries_for_hook(hook.id(), 10).await.unwrap().len(), 1);
        // The smaller id commits only now, after the larger one was delivered.
        insert(early, "refs/heads/early-id").await;
        poll(&s).await.unwrap();
        let ds = s.db.deliveries_for_hook(hook.id(), 10).await.unwrap();
        assert_eq!(ds.len(), 2, "the late commit is delivered too");
        assert!(ds.iter().any(|d| d.payload.contains("early-id")));
        assert!(ds.iter().any(|d| d.payload.contains("late-id")));
    }

    /// A ping, test or redelivery posts exactly the delivery it asked for: the backlog stays
    /// with the dispatcher, and a row another instance holds is not posted twice.
    #[tokio::test]
    async fn deliver_now_posts_only_the_requested_delivery() {
        let s = crate::testing::state("delivernow", |_| {}).await;
        let (u, _) = crate::testing::user_with_token(&s, "pinger").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        // A private target: the outbound guard refuses it without touching the network, so
        // each post attempt is recorded as a delivery error to retry.
        let hook = HookInput {
            url: "http://127.0.0.1:9/h".into(),
            content_type: "json".into(),
            events: vec!["*".into()],
            active: true,
            ..Default::default()
        };
        let hook = s.db.create_hook("repo", repo.id(), &hook).await.unwrap();
        let backlog = enqueue_one(&s, &hook, "push", None, &json!({}), Some(repo.id()), None).await.unwrap();
        let ping = enqueue_one(&s, &hook, "ping", None, &json!({"zen": "z"}), Some(repo.id()), None).await.unwrap();
        deliver_now(&s, &hook, ping.id()).await;
        let p = s.db.delivery_by_id(ping.id()).await.unwrap();
        assert_eq!(p.attempt, 1, "the requested delivery was posted");
        assert!(p.lock_until.is_none() && p.next_attempt_at.is_some(), "and released with a retry scheduled");
        assert!(p.response_body.as_deref().unwrap_or("").contains("delivery error"), "{:?}", p.response_body);
        let b = s.db.delivery_by_id(backlog.id()).await.unwrap();
        assert_eq!((b.attempt, b.lock_until, b.status.as_str()), (0, None, "pending"), "the backlog was not touched");
        // A row leased elsewhere is left alone.
        let held = s.db.claim_deliveries(10, LOCK_MS).await.unwrap();
        assert_eq!(held.iter().map(|d| d.id).collect::<Vec<_>>(), vec![backlog.id], "the ping waits for its retry time");
        deliver_now(&s, &hook, backlog.id()).await;
        let b2 = s.db.delivery_by_id(backlog.id()).await.unwrap();
        assert_eq!((b2.attempt, b2.lock_until), (1, held[0].lock_until), "another holder's lease is respected");
        assert!(s.db.claim_delivery(backlog.id(), LOCK_MS).await.unwrap().is_none());
    }

    #[test]
    fn signatures_match_github_examples() {
        // From GitHub's "Validating webhook deliveries" docs.
        let sig = signature_sha256("It's a Secret to Everybody", b"Hello, World!");
        assert_eq!(sig, "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17");
        let sig1 = signature_sha1("It's a Secret to Everybody", b"Hello, World!");
        assert!(sig1.starts_with("sha1=") && sig1.len() == 5 + 40);
    }

    #[test]
    fn form_hooks_wrap_the_json_in_payload() {
        let hook = Webhook {
            id: 1,
            owner_kind: "repo".into(),
            owner_id: 1,
            url: "http://x".into(),
            content_type: "form".into(),
            secret: None,
            events: "[\"*\"]".into(),
            active: 1,
            insecure_ssl: 0,
            last_status: None,
            last_delivered_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let (ct, body) = encode_body(&hook, "{\"a\":1}");
        assert_eq!(ct, "application/x-www-form-urlencoded");
        assert_eq!(String::from_utf8(body).unwrap(), "payload=%7B%22a%22%3A1%7D");
        assert!(hook.wants("push") && hook.wants("issues"));
    }
}
