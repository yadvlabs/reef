//! Repository insights and profile contributions: GitHub's `/stats/*` endpoints, the
//! pulse summary, last-commit-per-entry for tree listings, and the contribution calendar.
//!
//! Everything is computed from one cached commit index per repository (the default
//! branch's history from `git log --numstat`), refreshed when the branch tip moves.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiResult;
use crate::rest::repos::load;
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::repos::RepoWithOwner;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;
const MAX_COMMITS: usize = 20_000;
const INDEX_TTL: Duration = Duration::from_secs(600);
const CONTRIBUTIONS_TTL: Duration = Duration::from_secs(120);

/// GitHub-shaped `/repos/{owner}/{repo}/stats/*` (mounted under `/api/v3`). Timed like any
/// other route: the history walk behind them (`commit_index`) runs detached and is cached, so
/// a walk that outlives one request still completes and serves the next.
pub fn github_router() -> Router<AppState> {
    Router::new()
        .route("/repos/{owner}/{repo}/stats/contributors", get(contributors))
        .route("/repos/{owner}/{repo}/stats/commit_activity", get(commit_activity))
        .route("/repos/{owner}/{repo}/stats/code_frequency", get(code_frequency))
        .route("/repos/{owner}/{repo}/stats/participation", get(participation))
        .route("/repos/{owner}/{repo}/stats/punch_card", get(punch_card))
}

/// Reef-shaped insights (mounted under `/api/reef`); timed, see `github_router`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{owner}/{repo}/pulse", get(pulse))
        .route("/repos/{owner}/{repo}/tree-commits", get(tree_commits))
        .route("/users/{login}/contributions", get(contributions))
}

// ----- commit index -----

/// One non-merge commit on the default branch.
#[derive(Debug, Clone)]
pub struct CommitRec {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    /// Author time, Unix seconds.
    pub time: i64,
    pub additions: i64,
    pub deletions: i64,
    pub files: i64,
}

struct RepoIndex {
    head: String,
    commits: Arc<Vec<CommitRec>>,
    at: Instant,
}

/// Process-wide caches: per-repository commit indexes and rendered contribution calendars.
#[derive(Default)]
pub struct StatsCache {
    repos: Mutex<HashMap<i64, RepoIndex>>,
    /// History walks in flight, one per repository.
    walks: SingleFlight<i64, Arc<Vec<CommitRec>>>,
    contributions: Mutex<HashMap<String, (Instant, Value)>>,
}

type Flight<V> = futures::future::Shared<futures::future::BoxFuture<'static, Result<V, Arc<str>>>>;

/// At most one piece of background work in flight per key: the first caller starts it on its
/// own task (so it completes even when every caller stops waiting), later callers for the same
/// key wait for that one instead of starting another.
pub struct SingleFlight<K, V> {
    inflight: Arc<Mutex<HashMap<K, Flight<V>>>>,
    started: std::sync::atomic::AtomicU64,
}

impl<K, V> Default for SingleFlight<K, V> {
    fn default() -> Self {
        SingleFlight { inflight: Arc::new(Mutex::new(HashMap::new())), started: Default::default() }
    }
}

impl<K, V> SingleFlight<K, V>
where
    K: Eq + std::hash::Hash + Clone + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Wait for the work in flight for `key`, or start `work` when there is none.
    pub fn join<F>(&self, key: K, work: F) -> Flight<V>
    where
        F: Future<Output = Result<V, Error>> + Send + 'static,
    {
        use futures::FutureExt;
        let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = inflight.get(&key) {
            return f.clone();
        }
        self.started.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (map, k) = (self.inflight.clone(), key.clone());
        // The entry is inserted below while the lock is held, so the task's removal (which
        // needs the lock) always comes after it.
        let task = tokio::spawn(async move {
            let out = work.await.map_err(|e| Arc::<str>::from(e.to_string()));
            map.lock().unwrap_or_else(|e| e.into_inner()).remove(&k);
            out
        });
        let flight = async move { task.await.unwrap_or_else(|e| Err(Arc::from(format!("background work failed: {e}")))) }.boxed().shared();
        inflight.insert(key, flight.clone());
        flight
    }

    /// How many pieces of work were started (not joined).
    pub fn started(&self) -> u64 {
        self.started.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// The default branch's commit index for `rw`, from cache when the tip has not moved.
pub async fn commit_index(s: &AppState, rw: &RepoWithOwner) -> ApiResult<Arc<Vec<CommitRec>>> {
    if rw.repo.is_empty != 0 {
        return Ok(Arc::new(Vec::new()));
    }
    let path = s.store.path(rw.repo.id());
    let head = match reef_git::ops::rev_parse(s.store.git_binary(), &path, &rw.repo.default_branch).await {
        Ok(h) => h,
        Err(_) => return Ok(Arc::new(Vec::new())),
    };
    if let Ok(cache) = s.stats.repos.lock()
        && let Some(ix) = cache.get(&rw.repo.id)
        && ix.head == head
        && ix.at.elapsed() < INDEX_TTL
    {
        return Ok(ix.commits.clone());
    }
    // One walk per repository at a time, detached: a walk the request timeout cuts short still
    // fills the cache, and concurrent callers (a tree listing and the insights tabs opening
    // together, or a cold cache under load) wait for the walk in flight instead of starting
    // their own `git log` over the whole history.
    let (s2, rw) = (s.clone(), rw.clone());
    let walk = s.stats.walks.join(rw.repo.id, async move {
        let out = tokio::process::Command::new(s2.store.git_binary())
            .arg("-C")
            .arg(&path)
            .args([
                "log",
                "--no-merges",
                "--format=%x01%H%x00%an%x00%ae%x00%at",
                "--numstat",
                "-n",
                &MAX_COMMITS.to_string(),
                &rw.repo.default_branch,
                "--",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(Error::internal)?;
        let commits = Arc::new(parse_numstat_log(&String::from_utf8_lossy(&out.stdout)));
        if let Ok(mut cache) = s2.stats.repos.lock() {
            cache.insert(rw.repo.id, RepoIndex { head, commits: commits.clone(), at: Instant::now() });
        }
        Ok(commits)
    });
    Ok(walk.await.map_err(|e| Error::msg(e.to_string()))?)
}

fn parse_numstat_log(text: &str) -> Vec<CommitRec> {
    let mut out = Vec::new();
    for block in text.split('\u{1}').skip(1) {
        let mut lines = block.lines();
        let Some(header) = lines.next() else { continue };
        let mut f = header.split('\0');
        let (Some(sha), Some(name), Some(email), Some(at)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        let mut rec = CommitRec {
            sha: sha.to_string(),
            author_name: name.to_string(),
            author_email: email.to_string(),
            time: at.trim().parse().unwrap_or(0),
            additions: 0,
            deletions: 0,
            files: 0,
        };
        for line in lines {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.split('\t');
            let (Some(a), Some(d)) = (cols.next(), cols.next()) else { continue };
            rec.files += 1;
            rec.additions += a.parse::<i64>().unwrap_or(0);
            rec.deletions += d.parse::<i64>().unwrap_or(0);
        }
        out.push(rec);
    }
    out
}

// ----- authors -----

/// Who a commit belongs to: a Reef account, or an anonymous (email-keyed) author.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AuthorKey {
    Account(i64),
    Anonymous(String),
}

/// Resolves commit authors to accounts (by e-mail, then by name equal to a login), caching
/// lookups for the duration of one request.
struct AuthorResolver {
    by_email: HashMap<String, Option<Account>>,
    by_name: HashMap<String, Option<Account>>,
}

impl AuthorResolver {
    fn new() -> Self {
        AuthorResolver { by_email: HashMap::new(), by_name: HashMap::new() }
    }

    async fn resolve(&mut self, s: &AppState, c: &CommitRec) -> (AuthorKey, Option<Account>) {
        let email = c.author_email.to_lowercase();
        if !email.is_empty() {
            if !self.by_email.contains_key(&email) {
                let found = s.db.user_by_any_email(&email).await.ok().filter(|a| a.is_user());
                self.by_email.insert(email.clone(), found);
            }
            if let Some(a) = self.by_email.get(&email).cloned().flatten() {
                return (AuthorKey::Account(a.id), Some(a));
            }
        }
        let name = c.author_name.to_lowercase();
        if !name.is_empty() && reef_core::names::validate_login(&name).is_ok() {
            if !self.by_name.contains_key(&name) {
                let found = s.db.account_by_login(&name).await.ok().filter(|a| a.is_user());
                self.by_name.insert(name.clone(), found);
            }
            if let Some(a) = self.by_name.get(&name).cloned().flatten() {
                return (AuthorKey::Account(a.id), Some(a));
            }
        }
        (AuthorKey::Anonymous(if email.is_empty() { format!("name:{name}") } else { email }), None)
    }
}

fn author_json(s: &AppState, account: Option<&Account>, c: &CommitRec) -> Value {
    match account {
        Some(a) => shapes::simple_user(&s.config, a),
        None => json!({
            "login": Value::Null,
            "name": c.author_name,
            "email": c.author_email,
            "type": "Anonymous",
            "avatar_url": format!("{}/avatars/{}", s.config.site_url(), shapes::urlpath(&c.author_name)),
        }),
    }
}

// ----- calendar helpers -----

/// Days since the Unix epoch.
fn day_of(t: i64) -> i64 {
    t.div_euclid(DAY)
}

/// Weekday with Sunday = 0 (1970-01-01 was a Thursday).
fn weekday(t: i64) -> i64 {
    (day_of(t) + 4).rem_euclid(7)
}

/// Unix time of the Sunday 00:00 UTC that starts the week containing `t`.
fn week_start(t: i64) -> i64 {
    (day_of(t) - weekday(t)) * DAY
}

fn now_secs() -> i64 {
    reef_core::time::now_ms().div_euclid(1000)
}

fn date_string(day: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(day * DAY).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let d = t.date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

/// Parse `YYYY-MM-DD` into days since the epoch.
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u8 = it.next()?.parse().ok()?;
    let d: u8 = it.next()?.parse().ok()?;
    let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, d).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp().div_euclid(DAY))
}

// ----- GitHub stats -----

#[derive(Default)]
struct WeekTotals {
    a: i64,
    d: i64,
    c: i64,
}

async fn contributors(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let commits = commit_index(&s, &rw).await?;
    if commits.is_empty() {
        return Ok(Json(json!([])));
    }
    let first = commits.iter().map(|c| c.time).min().unwrap_or(0);
    let start = week_start(first);
    let end = week_start(now_secs());
    let mut resolver = AuthorResolver::new();
    let mut authors: Vec<(AuthorKey, Value, i64, BTreeMap<i64, WeekTotals>)> = Vec::new();
    let mut index: HashMap<AuthorKey, usize> = HashMap::new();
    for c in commits.iter() {
        let (key, account) = resolver.resolve(&s, c).await;
        let i = match index.get(&key) {
            Some(i) => *i,
            None => {
                authors.push((key.clone(), author_json(&s, account.as_ref(), c), 0, BTreeMap::new()));
                index.insert(key, authors.len() - 1);
                authors.len() - 1
            }
        };
        let entry = &mut authors[i];
        entry.2 += 1;
        let w = entry.3.entry(week_start(c.time)).or_default();
        w.a += c.additions;
        w.d += c.deletions;
        w.c += 1;
    }
    authors.sort_by_key(|a| std::cmp::Reverse(a.2));
    let out: Vec<Value> = authors
        .into_iter()
        .map(|(_, author, total, weeks)| {
            let mut ws = Vec::new();
            let mut w = start;
            while w <= end {
                let t = weeks.get(&w);
                ws.push(
                    json!({"w": w, "a": t.map(|t| t.a).unwrap_or(0), "d": t.map(|t| t.d).unwrap_or(0), "c": t.map(|t| t.c).unwrap_or(0)}),
                );
                w += WEEK;
            }
            json!({"author": author, "total": total, "weeks": ws})
        })
        .collect();
    Ok(Json(json!(out)))
}

async fn commit_activity(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let commits = commit_index(&s, &rw).await?;
    let end = week_start(now_secs());
    let start = end - 51 * WEEK;
    let mut weeks: BTreeMap<i64, [i64; 7]> = (0..52).map(|i| (start + i * WEEK, [0; 7])).collect();
    for c in commits.iter() {
        if c.time < start {
            continue;
        }
        if let Some(days) = weeks.get_mut(&week_start(c.time)) {
            days[weekday(c.time) as usize] += 1;
        }
    }
    let out: Vec<Value> = weeks.into_iter().map(|(w, days)| json!({"days": days, "total": days.iter().sum::<i64>(), "week": w})).collect();
    Ok(Json(json!(out)))
}

async fn code_frequency(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let commits = commit_index(&s, &rw).await?;
    if commits.is_empty() {
        return Ok(Json(json!([])));
    }
    let first = commits.iter().map(|c| c.time).min().unwrap_or(0);
    let start = week_start(first);
    let end = week_start(now_secs());
    let mut weeks: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut w = start;
    while w <= end {
        weeks.insert(w, (0, 0));
        w += WEEK;
    }
    for c in commits.iter() {
        if let Some(e) = weeks.get_mut(&week_start(c.time)) {
            e.0 += c.additions;
            e.1 += c.deletions;
        }
    }
    let out: Vec<Value> = weeks.into_iter().map(|(w, (a, d))| json!([w, a, -d])).collect();
    Ok(Json(json!(out)))
}

async fn participation(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let commits = commit_index(&s, &rw).await?;
    let end = week_start(now_secs());
    let start = end - 51 * WEEK;
    let mut all = [0i64; 52];
    let mut owner_counts = [0i64; 52];
    let mut resolver = AuthorResolver::new();
    for c in commits.iter() {
        if c.time < start {
            continue;
        }
        let i = ((week_start(c.time) - start) / WEEK) as usize;
        if i >= 52 {
            continue;
        }
        all[i] += 1;
        if let (AuthorKey::Account(id), _) = resolver.resolve(&s, c).await
            && id == rw.owner.id
        {
            owner_counts[i] += 1;
        }
    }
    Ok(Json(json!({"all": all.to_vec(), "owner": owner_counts.to_vec()})))
}

async fn punch_card(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let commits = commit_index(&s, &rw).await?;
    let mut cells = [[0i64; 24]; 7];
    for c in commits.iter() {
        let day = weekday(c.time) as usize;
        let hour = (c.time.rem_euclid(DAY) / 3600) as usize;
        cells[day][hour] += 1;
    }
    let mut out = Vec::with_capacity(168);
    for (day, hours) in cells.iter().enumerate() {
        for (hour, n) in hours.iter().enumerate() {
            out.push(json!([day, hour, n]));
        }
    }
    Ok(Json(json!(out)))
}

// ----- pulse -----

#[derive(serde::Deserialize, Default)]
pub struct PulseQuery {
    days: Option<i64>,
}

fn pulse_item(s: &AppState, rw: &RepoWithOwner, i: &reef_db::issues::Issue, user: Option<&Account>, merged_at: Option<i64>) -> Value {
    let kind = if i.is_pull() { "pull" } else { "issues" };
    json!({
        "number": i.number,
        "title": i.title,
        "html_url": format!("{}/{}/{}/{}", s.config.site_url(), rw.owner.login, rw.repo.name, format!("{kind}/{}", i.number)),
        "user": user.map(|u| shapes::simple_user(&s.config, u)),
        "created_at": iso(i.created_at),
        "closed_at": i.closed_at.map(iso),
        "merged_at": merged_at.map(iso),
    })
}

async fn pulse_items(
    s: &AppState,
    rw: &RepoWithOwner,
    list: Vec<(reef_db::issues::Issue, Option<i64>)>,
    users: &mut HashMap<i64, Option<Account>>,
) -> Vec<Value> {
    let mut out = Vec::with_capacity(list.len());
    for (i, merged_at) in list {
        if let std::collections::hash_map::Entry::Vacant(slot) = users.entry(i.author_id) {
            let u = s.db.account_by_id(Id(i.author_id)).await.ok();
            slot.insert(u);
        }
        out.push(pulse_item(s, rw, &i, users.get(&i.author_id).and_then(|u| u.as_ref()), merged_at));
    }
    out
}

async fn pulse(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<PulseQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let days = match q.days.unwrap_or(7) {
        d @ (1 | 3 | 7 | 30) => d,
        _ => 7,
    };
    let until_ms = reef_core::time::now_ms();
    let since_ms = until_ms - days * DAY * 1000;
    let since_secs = since_ms.div_euclid(1000);

    let commits = commit_index(&s, &rw).await?;
    let mut resolver = AuthorResolver::new();
    let mut by_author: Vec<(AuthorKey, Value, i64)> = Vec::new();
    let mut idx: HashMap<AuthorKey, usize> = HashMap::new();
    let (mut count, mut additions, mut deletions, mut files) = (0i64, 0i64, 0i64, 0i64);
    for c in commits.iter().filter(|c| c.time >= since_secs) {
        count += 1;
        additions += c.additions;
        deletions += c.deletions;
        files += c.files;
        let (key, account) = resolver.resolve(&s, c).await;
        match idx.get(&key) {
            Some(i) => by_author[*i].2 += 1,
            None => {
                by_author.push((key.clone(), author_json(&s, account.as_ref(), c), 1));
                idx.insert(key, by_author.len() - 1);
            }
        }
    }
    by_author.sort_by_key(|a| std::cmp::Reverse(a.2));
    let authors: Vec<Value> = by_author.iter().map(|(_, a, n)| json!({"author": a, "commits": n})).collect();

    let mut users: HashMap<i64, Option<Account>> = HashMap::new();
    let merged = s.db.pulls_merged_since(rw.repo.id(), since_ms).await?;
    let merged_pulls = pulse_items(&s, &rw, merged.into_iter().map(|m| (m.issue, Some(m.merged_at))).collect(), &mut users).await;
    let opened = s.db.issues_created_since(rw.repo.id(), since_ms, true).await?;
    let open_pulls = pulse_items(&s, &rw, opened.into_iter().filter(|i| i.is_open()).map(|i| (i, None)).collect(), &mut users).await;
    let closed = s.db.issues_closed_since(rw.repo.id(), since_ms, false).await?;
    let closed_issues = pulse_items(&s, &rw, closed.into_iter().map(|i| (i, None)).collect(), &mut users).await;
    let new = s.db.issues_created_since(rw.repo.id(), since_ms, false).await?;
    let new_issues = pulse_items(&s, &rw, new.into_iter().map(|i| (i, None)).collect(), &mut users).await;

    let active_pulls = {
        let mut seen = HashSet::new();
        for v in merged_pulls.iter().chain(open_pulls.iter()) {
            seen.insert(v["number"].as_i64().unwrap_or(0));
        }
        seen.len()
    };
    let active_issues = {
        let mut seen = HashSet::new();
        for v in closed_issues.iter().chain(new_issues.iter()) {
            seen.insert(v["number"].as_i64().unwrap_or(0));
        }
        seen.len()
    };
    Ok(Json(json!({
        "since": iso(since_ms),
        "until": iso(until_ms),
        "days": days,
        "commits": {"count": count, "authors": by_author.len(), "additions": additions, "deletions": deletions, "files_changed": files},
        "authors": authors,
        "merged_pulls": merged_pulls,
        "open_pulls": open_pulls,
        "closed_issues": closed_issues,
        "new_issues": new_issues,
        "counts": {
            "merged_pulls": merged_pulls.len(),
            "open_pulls": open_pulls.len(),
            "closed_issues": closed_issues.len(),
            "new_issues": new_issues.len(),
            "active_pulls": active_pulls,
            "active_issues": active_issues,
        },
    })))
}

// ----- tree commits -----

#[derive(serde::Deserialize, Default)]
pub struct TreeCommitsQuery {
    rev: Option<String>,
    path: Option<String>,
}

/// Latest commit touching each entry of a directory, plus the commit count of `rev`.
async fn tree_commits(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<TreeCommitsQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty != 0 {
        return Ok(Json(json!({"commits": 0, "entries": {}})));
    }
    let rev = q.rev.filter(|r| !r.is_empty()).unwrap_or_else(|| rw.repo.default_branch.clone());
    let dir = q.path.unwrap_or_default().trim_matches('/').to_string();
    let path = s.store.path(rw.repo.id());
    // Refuse revisions that look like options.
    if rev.starts_with('-') {
        return Err(crate::error::ApiError(Error::NotFound));
    }
    let (rev2, dir2) = (rev.clone(), dir.clone());
    let names: HashSet<String> =
        crate::rest::contents::with_repo(&s, &rw, move |r| Ok(r.tree(&rev2, &dir2)?.into_iter().map(|e| e.name).collect())).await?;

    let count_out = tokio::process::Command::new(s.store.git_binary())
        .arg("-C")
        .arg(&path)
        .args(["rev-list", "--count", &rev, "--"])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(Error::internal)?;
    let total: i64 = String::from_utf8_lossy(&count_out.stdout).trim().parse().unwrap_or(0);

    let mut args: Vec<String> = vec![
        "log".into(),
        rev.clone(),
        "--format=%x01%H%x00%an%x00%ae%x00%at%x00%s".into(),
        "--name-only".into(),
        "-n".into(),
        "3000".into(),
        "--".into(),
    ];
    if !dir.is_empty() {
        args.push(format!("{dir}/"));
    }
    let out = tokio::process::Command::new(s.store.git_binary())
        .arg("-C")
        .arg(&path)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(Error::internal)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };

    let mut resolver = AuthorResolver::new();
    let mut entries = serde_json::Map::new();
    let mut remaining = names.clone();
    'blocks: for block in text.split('\u{1}').skip(1) {
        let mut lines = block.lines();
        let Some(header) = lines.next() else { continue };
        let mut f = header.split('\0');
        let (Some(sha), Some(name), Some(email), Some(at), subject) = (f.next(), f.next(), f.next(), f.next(), f.next().unwrap_or(""))
        else {
            continue;
        };
        let rec = CommitRec {
            sha: sha.to_string(),
            author_name: name.to_string(),
            author_email: email.to_string(),
            time: at.trim().parse().unwrap_or(0),
            additions: 0,
            deletions: 0,
            files: 0,
        };
        let mut resolved: Option<Value> = None;
        for file in lines {
            let file = file.trim();
            if file.is_empty() {
                continue;
            }
            let Some(rest) = file.strip_prefix(prefix.as_str()) else { continue };
            let entry = rest.split('/').next().unwrap_or(rest);
            if !remaining.contains(entry) {
                continue;
            }
            if resolved.is_none() {
                let (_, account) = resolver.resolve(&s, &rec).await;
                resolved = Some(json!({
                    "sha": rec.sha,
                    "message": subject,
                    "date": iso(rec.time * 1000),
                    "author": account.as_ref().map(|a| shapes::simple_user(&s.config, a)),
                    "author_name": rec.author_name,
                }));
            }
            entries.insert(entry.to_string(), resolved.clone().unwrap_or(Value::Null));
            remaining.remove(entry);
            if remaining.is_empty() {
                break 'blocks;
            }
        }
    }
    Ok(Json(json!({"commits": total, "entries": Value::Object(entries)})))
}

// ----- contributions -----

#[derive(serde::Deserialize, Default)]
pub struct ContributionsQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Default)]
struct DayCounts {
    commits: i64,
    issues: i64,
    pulls: i64,
}

#[derive(Default)]
struct MonthCounts {
    commits: BTreeMap<String, i64>,
    issues: BTreeMap<String, i64>,
    pulls: BTreeMap<String, i64>,
}

/// Contribution calendar and monthly activity for a user, over the repositories the
/// viewer can read.
async fn contributions(
    State(s): State<AppState>,
    auth: Auth,
    Path(login): Path<String>,
    Query(q): Query<ContributionsQuery>,
) -> ApiResult<Json<Value>> {
    let account = s.db.account_by_login(&login).await?;
    if !account.is_user() {
        return Err(crate::error::ApiError(Error::NotFound));
    }
    let today = day_of(now_secs());
    let to_day = q.to.as_deref().and_then(parse_day).unwrap_or(today);
    let from_day = q.from.as_deref().and_then(parse_day).unwrap_or(to_day - 364);
    if from_day > to_day || to_day - from_day > 5 * 366 {
        return Err(crate::error::ApiError(Error::validation("invalid date range")));
    }
    let viewer = auth.user.as_ref().map(|u| u.id).unwrap_or(0);
    let key = format!("{}|{viewer}|{from_day}|{to_day}", account.login.to_lowercase());
    if let Ok(cache) = s.stats.contributions.lock()
        && let Some((at, v)) = cache.get(&key)
        && at.elapsed() < CONTRIBUTIONS_TTL
    {
        return Ok(Json(v.clone()));
    }

    let since_secs = from_day * DAY;
    let until_secs = (to_day + 1) * DAY - 1;
    let email = account.email.clone().unwrap_or_default();
    let login_lower = account.login.to_lowercase();
    let mut days: BTreeMap<i64, DayCounts> = (from_day..=to_day).map(|d| (d, DayCounts::default())).collect();
    let mut months: BTreeMap<String, MonthCounts> = BTreeMap::new();

    let repos = s.db.all_repos(0, 10_000).await?;
    let mut readable: HashMap<i64, String> = HashMap::new();
    for rw in &repos {
        if !s.db.permission(auth.user.as_ref(), rw).await?.can_read() {
            continue;
        }
        readable.insert(rw.repo.id, rw.full_name());
        if rw.repo.is_empty != 0 {
            continue;
        }
        let commits = commit_index(&s, rw).await?;
        for c in commits.iter() {
            if c.time < since_secs || c.time > until_secs {
                continue;
            }
            let mine =
                (!email.is_empty() && c.author_email.eq_ignore_ascii_case(&email)) || c.author_name.eq_ignore_ascii_case(&login_lower);
            if !mine {
                continue;
            }
            let d = day_of(c.time);
            if let Some(e) = days.get_mut(&d) {
                e.commits += 1;
            }
            let month = date_string(d)[..7].to_string();
            *months.entry(month).or_default().commits.entry(rw.full_name()).or_default() += 1;
        }
    }
    for item in s.db.issues_by_author_since(account.id(), since_secs * 1000, until_secs * 1000 + 999).await? {
        let Some(full) = readable.get(&item.repo_id) else { continue };
        let d = day_of(item.created_at.div_euclid(1000));
        let month = date_string(d)[..7].to_string();
        let m = months.entry(month).or_default();
        if item.is_pull {
            if let Some(e) = days.get_mut(&d) {
                e.pulls += 1;
            }
            *m.pulls.entry(full.clone()).or_default() += 1;
        } else {
            if let Some(e) = days.get_mut(&d) {
                e.issues += 1;
            }
            *m.issues.entry(full.clone()).or_default() += 1;
        }
    }

    let mut total = 0;
    let day_list: Vec<Value> = days
        .iter()
        .map(|(d, c)| {
            let n = c.commits + c.issues + c.pulls;
            total += n;
            json!({"date": date_string(*d), "count": n, "commits": c.commits, "issues": c.issues, "pulls": c.pulls})
        })
        .collect();
    let month_list: Vec<Value> = months
        .iter()
        .rev()
        .filter(|(_, m)| !(m.commits.is_empty() && m.issues.is_empty() && m.pulls.is_empty()))
        .map(|(month, m)| {
            let list = |b: &BTreeMap<String, i64>| {
                let mut v: Vec<(&String, &i64)> = b.iter().collect();
                v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                v.into_iter().map(|(repo, n)| json!({"repo": repo, "count": n})).collect::<Vec<_>>()
            };
            json!({"month": month, "commits": list(&m.commits), "issues": list(&m.issues), "pulls": list(&m.pulls)})
        })
        .collect();
    let out = json!({"from": date_string(from_day), "to": date_string(to_day), "total": total, "days": day_list, "months": month_list});
    if let Ok(mut cache) = s.stats.contributions.lock() {
        cache.retain(|_, (at, _)| at.elapsed() < CONTRIBUTIONS_TTL);
        cache.insert(key, (Instant::now(), out.clone()));
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_sunday() {
        // 2026-09-27 is a Sunday.
        let sunday = parse_day("2026-09-27").unwrap() * DAY;
        assert_eq!(weekday(sunday), 0);
        assert_eq!(week_start(sunday + 3 * DAY + 100), sunday);
        assert_eq!(date_string(day_of(sunday)), "2026-09-27");
    }

    /// Concurrent callers share the walk in flight; a later one starts a new walk.
    #[tokio::test]
    async fn one_walk_per_repository_is_in_flight() {
        let flights: Arc<SingleFlight<i64, Arc<Vec<CommitRec>>>> = Arc::default();
        let runs = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (release, released) = tokio::sync::watch::channel(false);
        let work = || {
            let (runs, mut released) = (runs.clone(), released.clone());
            async move {
                runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = released.wait_for(|r| *r).await;
                Ok(Arc::new(Vec::new()))
            }
        };
        let waiters: Vec<_> = (0..8).map(|_| tokio::spawn(flights.join(7, work()))).collect();
        let other = tokio::spawn(flights.join(8, work()));
        tokio::task::yield_now().await;
        assert_eq!(flights.started(), 2, "one walk for repository 7, one for 8");
        release.send(true).unwrap();
        for w in waiters {
            assert!(w.await.unwrap().is_ok());
        }
        assert!(other.await.unwrap().is_ok());
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Done: the next caller starts afresh (the cache in front of this answers repeats).
        assert!(flights.join(7, work()).await.is_ok());
        assert_eq!(flights.started(), 3);
        // A failing walk reaches every waiter as an error and is not kept.
        let failing = flights.join(9, async { Err(Error::msg("git log failed")) });
        assert_eq!(failing.clone().await.unwrap_err().as_ref(), "git log failed");
        assert!(flights.join(9, work()).await.is_ok());
    }

    #[test]
    fn parses_numstat_blocks() {
        let text = "\u{1}abc\0Sam\0sam@example.com\u{0}1700000000\n\n3\t1\tsrc/a.rs\n-\t-\tbin\n\u{1}def\0Priya\0p@example.com\u{0}1700000100\n\n1\t0\tREADME.md\n";
        let recs = parse_numstat_log(text);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].additions, 3);
        assert_eq!(recs[0].deletions, 1);
        assert_eq!(recs[0].files, 2);
        assert_eq!(recs[1].author_name, "Priya");
    }
}
