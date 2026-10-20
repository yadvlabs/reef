//! Retention and cleanup, run hourly on every instance (`run`): workflow runs that completed
//! longer than `[actions] retention` ago (or the site setting `actions_retention_days`) are removed with
//! their jobs, logs and artifacts; artifacts past their own `expires_at` lose their file and
//! are marked expired (GitHub keeps listing them); scratch files under `<data_dir>/tmp`
//! older than a day are deleted; and `reef_actions_logs_bytes` / `reef_actions_artifacts_bytes`
//! are recomputed. Every step is idempotent and safe to run on several instances at once:
//! files are removed before rows, so a pass cut short leaves rows the next pass finishes.

use crate::AppState;
use reef_core::{Error, Id, Result};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Files under `<data_dir>/tmp` untouched for this long are removed.
pub const TMP_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// How often `run` makes a pass.
pub const INTERVAL: Duration = Duration::from_secs(3600);
const BATCH: i64 = 200;

/// What one pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub runs_removed: u64,
    pub artifacts_expired: u64,
    pub tmp_removed: u64,
}

/// The longest retention honoured (100 years). Larger values are clamped so the cutoff
/// arithmetic can never overflow into "purge everything".
pub const MAX_RETENTION_DAYS: i64 = 36_500;

const DAY_MS: i64 = 86_400_000;
/// `MAX_RETENTION_DAYS` in milliseconds.
pub const MAX_RETENTION_MS: i64 = MAX_RETENTION_DAYS * DAY_MS;

/// The retention in milliseconds: `[actions] retention` (a humantime duration), or the site
/// console's `actions_retention_days` (whole days) when set and a number; clamped to
/// `0..=MAX_RETENTION_MS`. `0` keeps runs forever.
pub async fn retention_ms(s: &AppState) -> i64 {
    let configured = i64::try_from(s.config.actions.retention().as_millis()).unwrap_or(i64::MAX);
    let console = s.db.setting("actions_retention_days").await.ok().flatten().and_then(|v| v.trim().parse::<i64>().ok());
    match console {
        Some(days) => days.clamp(0, MAX_RETENTION_DAYS) * DAY_MS,
        None => configured,
    }
    .clamp(0, MAX_RETENTION_MS)
}

/// The purge cutoff (epoch ms) for `retention` ms as of `now`: `None` keeps everything
/// (retention off, or a cutoff that would not be a real point in time).
pub fn cutoff_ms(now: i64, retention: i64) -> Option<i64> {
    if retention <= 0 {
        return None;
    }
    let cutoff = now.saturating_sub(retention.min(MAX_RETENTION_MS));
    (cutoff > 0).then_some(cutoff)
}

/// Remove completed runs that finished before the retention window (a re-run finishes again,
/// so it is kept), files first. A run that cannot be removed is logged and skipped: the pass
/// walks a cursor past it, so it never holds up the runs behind it, and the next pass retries.
pub async fn purge_runs(s: &AppState) -> Result<u64> {
    let Some(cutoff) = cutoff_ms(reef_core::time::now_ms(), retention_ms(s).await) else { return Ok(0) };
    let mut removed = 0;
    let mut after = (i64::MIN, i64::MIN);
    loop {
        let runs = s.db.runs_completed_before(cutoff, after, BATCH).await?;
        let n = runs.len() as i64;
        for r in runs {
            after = (r.completed_at.unwrap_or(r.updated_at), r.id);
            s.actions.files.remove_run(r.id).await;
            match s.db.delete_run(r.id()).await {
                Ok(()) => removed += 1,
                Err(Error::NotFound) => {} // a sibling instance got there first
                Err(e) => tracing::warn!(run = r.id, error = %e, "retention: removing a run failed; will retry"),
            }
        }
        if n < BATCH {
            break;
        }
    }
    Ok(removed)
}

/// How long an artifact whose expiry failed waits before the next attempt.
pub const ARTIFACT_RETRY: Duration = INTERVAL;

/// Drop the files of artifacts past `expires_at` and mark them expired. A row whose file
/// cannot be removed is logged, skipped for the rest of the pass (the cursor moves past it)
/// and marked to be retried after `ARTIFACT_RETRY`, so it never blocks the ones behind it.
pub async fn expire_artifacts(s: &AppState) -> Result<u64> {
    let now = reef_core::time::now_ms();
    let mut expired = 0;
    let mut after = (i64::MIN, i64::MIN);
    loop {
        let list = s.db.artifacts_expired_at(now, after, BATCH).await?;
        let n = list.len() as i64;
        for a in list {
            after = (a.expires_at.unwrap_or(i64::MIN), a.id);
            let result = match &a.path {
                Some(p) => s.actions.files.remove(p).await.map_err(|e| e.to_string()),
                None => Ok(()),
            };
            let result = match result {
                Ok(()) => s.db.expire_artifact(Id(a.id)).await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            match result {
                Ok(()) => expired += 1,
                Err(e) => {
                    tracing::warn!(artifact = a.id, error = %e, "retention: expiring an artifact failed; will retry later");
                    let _ = s.db.defer_artifact_expiry(Id(a.id), now + ARTIFACT_RETRY.as_millis() as i64).await;
                }
            }
        }
        if n < BATCH {
            break;
        }
    }
    Ok(expired)
}

/// Remove entries of `dir` (files, or whole directories) whose modification time is older
/// than `max_age`. Returns how many were removed.
pub fn sweep_tmp(dir: &Path, max_age: Duration) -> u64 {
    let cutoff = SystemTime::now().checked_sub(max_age).unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let old = md.modified().map(|m| m < cutoff).unwrap_or(false);
        if !old {
            continue;
        }
        let ok = if md.is_dir() { std::fs::remove_dir_all(e.path()).is_ok() } else { std::fs::remove_file(e.path()).is_ok() };
        if ok {
            removed += 1;
        }
    }
    removed
}

/// One pass of everything.
pub async fn run_once(s: &AppState) -> Report {
    let mut r = Report::default();
    match purge_runs(s).await {
        Ok(n) => r.runs_removed = n,
        Err(e) => tracing::warn!(error = %e, "retention: purging runs failed"),
    }
    match expire_artifacts(s).await {
        Ok(n) => r.artifacts_expired = n,
        Err(e) => tracing::warn!(error = %e, "retention: expiring artifacts failed"),
    }
    let tmp = s.config.tmp_dir();
    r.tmp_removed = tokio::task::spawn_blocking(move || sweep_tmp(&tmp, TMP_MAX_AGE)).await.unwrap_or(0);
    crate::metrics::registry().actions_runs_purged.inc_by(r.runs_removed);
    crate::metrics::refresh_actions_bytes(s).await;
    if r.runs_removed + r.artifacts_expired + r.tmp_removed > 0 {
        tracing::info!(runs = r.runs_removed, artifacts = r.artifacts_expired, tmp_files = r.tmp_removed, "retention pass");
    }
    r
}

/// A pass at start, then every `INTERVAL`.
pub async fn run(s: AppState) {
    loop {
        run_once(&s).await;
        tokio::time::sleep(INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::logs::Files;
    use reef_db::actions::{NewJob, NewRun};
    use reef_db::repos::NewRepo;
    use reef_db::sqlx;

    #[tokio::test]
    async fn old_runs_expired_artifacts_and_stale_tmp_files_are_removed() {
        let s = crate::testing::state("retention", |c| c.actions.retention = "30d".into()).await;
        let (u, _) = crate::testing::user_with_token(&s, "keeper").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        let wf = s.db.upsert_workflow(repo.id(), ".github/workflows/ci.yml", "ci").await.unwrap();
        let mk_run = |s: &AppState| {
            let (db, repo, wf) = (s.db.clone(), repo.id(), wf.id());
            async move {
                db.create_run(NewRun {
                    repo,
                    workflow: wf,
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
                .unwrap()
            }
        };
        let old = mk_run(&s).await;
        let fresh = mk_run(&s).await;
        let labels = vec!["self-hosted".to_string()];
        let job =
            s.db.create_job(
                NewJob {
                    run: old.id(),
                    repo: repo.id(),
                    job_key: "build",
                    name: "build",
                    matrix: None,
                    needs: &[],
                    labels: &labels,
                    status: "queued",
                    order_index: 0,
                },
                1,
            )
            .await
            .unwrap();
        let log = s.db.create_job_log(job.id(), "").await.unwrap();
        let log_rel = Files::log_rel(old.id, job.id, log.id);
        s.db.set_log_path(log.id, &log_rel).await.unwrap();
        s.actions.files.write(&log_rel, b"hello\n").await.unwrap();
        let art_old = s.db.create_artifact(old.id(), repo.id(), "dist", 30).await.unwrap();
        let art_rel = Files::artifact_rel(old.id, art_old.id);
        s.actions.files.write(&art_rel, b"zip").await.unwrap();
        s.db.finalize_artifact(Id(art_old.id), &art_rel, 3).await.unwrap();
        // A fresh run with an artifact whose own expiry has passed.
        let art_expired = s.db.create_artifact(fresh.id(), repo.id(), "expired", 1).await.unwrap();
        let exp_rel = Files::artifact_rel(fresh.id, art_expired.id);
        s.actions.files.write(&exp_rel, b"zipzip").await.unwrap();
        s.db.finalize_artifact(Id(art_expired.id), &exp_rel, 6).await.unwrap();
        sqlx::query("UPDATE artifacts SET expires_at = 1 WHERE id = $1").bind(art_expired.id).execute(s.db.pool()).await.unwrap();
        // Age the old run past the window (completed 40 days ago); the fresh one stays queued.
        let forty_days_ago = reef_core::time::now_ms() - 40 * 86_400_000;
        sqlx::query("UPDATE workflow_runs SET created_at = $2, updated_at = $2, completed_at = $2, status = 'completed', conclusion = 'success' WHERE id = $1")
            .bind(old.id)
            .bind(forty_days_ago)
            .execute(s.db.pool())
            .await
            .unwrap();
        // Scratch files: one stale, one current.
        let tmp = s.config.tmp_dir();
        std::fs::write(tmp.join("stale"), b"x").unwrap();
        std::fs::create_dir_all(tmp.join("stale-dir")).unwrap();
        std::fs::write(tmp.join("stale-dir/f"), b"x").unwrap();
        std::fs::write(tmp.join("current"), b"x").unwrap();
        let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 86_400);
        for p in [tmp.join("stale"), tmp.join("stale-dir")] {
            std::fs::File::options().write(true).open(&p).or_else(|_| std::fs::File::open(&p)).unwrap().set_modified(two_days_ago).unwrap();
        }
        assert_eq!(retention_ms(&s).await, 30 * DAY_MS);

        let r = run_once(&s).await;
        assert_eq!(r, Report { runs_removed: 1, artifacts_expired: 1, tmp_removed: 2 }, "{r:?}");
        assert!(matches!(s.db.run_by_id(old.id()).await, Err(Error::NotFound)), "the old run is gone");
        assert!(s.db.run_by_id(fresh.id()).await.is_ok(), "the fresh run stays");
        assert!(s.db.job_by_id(job.id()).await.is_err());
        assert!(s.db.job_log(log.id).await.is_err());
        assert!(!s.actions.files.abs(&log_rel).exists(), "log file removed");
        assert!(!s.actions.files.abs(&art_rel).exists(), "artifact of the old run removed");
        assert!(matches!(s.db.artifact_by_id(Id(art_old.id)).await, Err(Error::NotFound)));
        let a = s.db.artifact_by_id(Id(art_expired.id)).await.unwrap();
        assert_eq!((a.expired, a.path.as_deref(), a.size), (1, None, 0), "expired artifacts keep their row");
        assert!(!s.actions.files.abs(&exp_rel).exists());
        assert!(!tmp.join("stale").exists() && !tmp.join("stale-dir").exists());
        assert!(tmp.join("current").exists(), "recent scratch files are kept");
        // A second pass finds nothing; the byte gauges were refreshed.
        assert_eq!(run_once(&s).await, Report::default());
        assert_eq!(crate::metrics::registry().actions_logs_bytes.get(), 0);
        // The console override wins, and 0 keeps everything.
        s.db.set_setting("actions_retention_days", Some("0"), None).await.unwrap();
        assert_eq!(retention_ms(&s).await, 0);
        sqlx::query("UPDATE workflow_runs SET created_at = $2, updated_at = $2, completed_at = $2, status = 'completed' WHERE id = $1")
            .bind(fresh.id)
            .bind(forty_days_ago)
            .execute(s.db.pool())
            .await
            .unwrap();
        assert_eq!(purge_runs(&s).await.unwrap(), 0);
        assert!(s.db.run_by_id(Id(fresh.id)).await.is_ok());
        // Absurd or negative values from the console are clamped, never overflowed.
        s.db.set_setting("actions_retention_days", Some("99999999999999999"), None).await.unwrap();
        assert_eq!(retention_ms(&s).await, MAX_RETENTION_MS);
        s.db.set_setting("actions_retention_days", Some("-5"), None).await.unwrap();
        assert_eq!(retention_ms(&s).await, 0);
        s.db.set_setting("actions_retention_days", Some("not a number"), None).await.unwrap();
        assert_eq!(retention_ms(&s).await, 30 * DAY_MS, "unparseable: the file's value");
    }

    fn new_run<'a>(repo: Id, wf: Id) -> NewRun<'a> {
        NewRun {
            repo,
            workflow: wf,
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
        }
    }

    /// Runs age by when they last completed: one created long ago but re-run inside the
    /// window stays; one that completed long ago goes.
    #[tokio::test]
    async fn reruns_inside_the_window_are_kept() {
        let s = crate::testing::state("retention-rerun", |c| c.actions.retention = "30d".into()).await;
        let (u, _) = crate::testing::user_with_token(&s, "rerunner").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        let wf = s.db.upsert_workflow(repo.id(), ".github/workflows/ci.yml", "ci").await.unwrap();
        let rerun = s.db.create_run(new_run(repo.id(), wf.id())).await.unwrap();
        let stale = s.db.create_run(new_run(repo.id(), wf.id())).await.unwrap();
        let now = reef_core::time::now_ms();
        let day = 86_400_000i64;
        for (id, completed) in [(rerun.id, now - day), (stale.id, now - 35 * day)] {
            sqlx::query("UPDATE workflow_runs SET created_at = $2, updated_at = $3, completed_at = $3, status = 'completed', conclusion = 'success' WHERE id = $1")
                .bind(id)
                .bind(now - 60 * day)
                .bind(completed)
                .execute(s.db.pool())
                .await
                .unwrap();
        }
        assert_eq!(purge_runs(&s).await.unwrap(), 1);
        assert!(s.db.run_by_id(rerun.id()).await.is_ok(), "re-run yesterday: kept");
        assert!(matches!(s.db.run_by_id(stale.id()).await, Err(Error::NotFound)), "completed 35 days ago: removed");
    }

    /// A full batch of artifacts whose files cannot be removed does not stop the pass: the
    /// cursor moves past them, they are retried after `ARTIFACT_RETRY`, and the artifact
    /// behind them expires.
    #[tokio::test]
    async fn failing_artifacts_do_not_block_the_rest() {
        let s = crate::testing::state("retention-stuck", |_| {}).await;
        let (u, _) = crate::testing::user_with_token(&s, "stuck").await;
        let repo = s.db.create_repo(u.id(), NewRepo { name: "r".into(), ..Default::default() }).await.unwrap();
        let wf = s.db.upsert_workflow(repo.id(), ".github/workflows/ci.yml", "ci").await.unwrap();
        let run = s.db.create_run(new_run(repo.id(), wf.id())).await.unwrap();
        // The failing ones: their "file" is a non-empty directory, which remove_file refuses.
        let mut stuck = Vec::new();
        for i in 0..=BATCH {
            let a = s.db.create_artifact(run.id(), repo.id(), &format!("stuck-{i}"), 1).await.unwrap();
            let rel = Files::artifact_rel(run.id, a.id);
            s.actions.files.write(&format!("{rel}/inner"), b"x").await.unwrap();
            s.db.finalize_artifact(Id(a.id), &rel, 1).await.unwrap();
            stuck.push(a.id);
        }
        let good = s.db.create_artifact(run.id(), repo.id(), "good", 1).await.unwrap();
        let good_rel = Files::artifact_rel(run.id, good.id);
        s.actions.files.write(&good_rel, b"zip").await.unwrap();
        s.db.finalize_artifact(Id(good.id), &good_rel, 3).await.unwrap();
        sqlx::query("UPDATE artifacts SET expires_at = 1 WHERE id <> $1").bind(good.id).execute(s.db.pool()).await.unwrap();
        sqlx::query("UPDATE artifacts SET expires_at = 2 WHERE id = $1").bind(good.id).execute(s.db.pool()).await.unwrap();

        assert_eq!(expire_artifacts(&s).await.unwrap(), 1, "the artifact behind a full batch of failures expires");
        assert_eq!(s.db.artifact_by_id(Id(good.id)).await.unwrap().expired, 1);
        let deferred: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM artifacts WHERE retry_after IS NOT NULL").fetch_one(s.db.pool()).await.unwrap();
        assert_eq!(deferred, stuck.len() as i64, "every failure is marked to retry later");
        assert!(
            s.db.artifacts_expired_at(reef_core::time::now_ms(), (i64::MIN, i64::MIN), 10).await.unwrap().is_empty(),
            "and waits it out"
        );
        // Once the retry time has passed and the file can go, it expires.
        sqlx::query("UPDATE artifacts SET retry_after = 1").execute(s.db.pool()).await.unwrap();
        std::fs::remove_dir_all(s.actions.files.abs(&Files::artifact_rel(run.id, stuck[0]))).unwrap();
        assert!(expire_artifacts(&s).await.unwrap() >= 1);
        assert_eq!(s.db.artifact_by_id(Id(stuck[0])).await.unwrap().expired, 1);
    }

    #[test]
    fn the_cutoff_never_overflows() {
        assert_eq!(cutoff_ms(1_000_000_000_000, 0), None);
        assert_eq!(cutoff_ms(1_000_000_000_000, -1), None);
        assert_eq!(cutoff_ms(1_000_000_000_000, DAY_MS), Some(1_000_000_000_000 - DAY_MS));
        assert_eq!(cutoff_ms(1_000_000_000_000, 12 * 3_600_000), Some(1_000_000_000_000 - 12 * 3_600_000), "sub-day retention");
        assert_eq!(cutoff_ms(1_000_000_000_000, i64::MAX), None, "a huge retention keeps everything instead of purging it");
        assert_eq!(cutoff_ms(i64::MIN, 30 * DAY_MS), None);
        assert!(cutoff_ms(reef_core::time::now_ms(), 365 * DAY_MS).is_some());
        assert_eq!(cutoff_ms(reef_core::time::now_ms(), MAX_RETENTION_MS), None, "a cutoff before the epoch keeps everything");
    }
}
