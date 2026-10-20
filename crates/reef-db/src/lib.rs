//! Database access for Reef. One code path serves SQLite and PostgreSQL: the backend is
//! chosen from the connection URL at runtime and the SQL is written to run on both.

pub use sqlx;

pub mod accounts;
pub mod actions;
pub mod auth;
pub mod device;
pub mod emails;
pub mod events;
pub mod gpg;
pub mod issues;
pub mod lfs;
pub mod mirrors;
pub mod notifications;
pub mod orgs;
pub mod projects;
pub mod protection;
pub mod pulls;
pub mod releases;
pub mod rename;
pub mod repos;
pub mod social;
pub mod stats;
pub mod webhooks;

use reef_core::Error;
use sqlx::AnyConnection;
use sqlx::AnyPool;
use sqlx::Connection;
use sqlx::any::AnyPoolOptions;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    pub fn from_url(url: &str) -> Result<Self, Error> {
        if url.starts_with("sqlite:") {
            Ok(Backend::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Backend::Postgres)
        } else {
            Err(Error::msg(format!("unsupported database url scheme: {url}")))
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Backend::Sqlite => "sqlite",
            Backend::Postgres => "postgres",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Db {
    pool: AnyPool,
    backend: Backend,
    /// The normalized connection URL, for the health pool.
    url: String,
    /// One connection kept open for readiness probes (`ping_dedicated`), apart from the
    /// application pool; created on the first probe, so short-lived helpers never open it.
    health: std::sync::Arc<tokio::sync::OnceCell<AnyPool>>,
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The schema versions (highest applied migration) this binary runs against. Expand and
/// contract: a release may add to the schema, and only drop what the previous release no
/// longer uses, so binary N runs against schema N+1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaRange {
    pub min: i64,
    pub max: i64,
}

impl SchemaRange {
    pub fn contains(&self, v: i64) -> bool {
        (self.min..=self.max).contains(&v)
    }
}

/// What this build supports: its own schema (migration 1) and the next expansion (2).
pub const SCHEMA_RANGE: SchemaRange = SchemaRange { min: 1, max: 2 };

/// The version this build migrates to.
pub fn latest_migration() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainState {
    /// No migration applied yet.
    Fresh,
    /// Only versions this build embeds are applied (some may still be pending).
    Known,
    /// Every embedded version is applied and a newer build added more within `SCHEMA_RANGE`.
    ExpandedByNewerBuild,
}

fn mismatch_message(version: i64) -> String {
    format!(
        "migration {version} was applied by a different build of Reef (its contents differ from this build's); discard the database (make reset in development) or run the build that created it"
    )
}

impl Db {
    /// Connect and apply pending migrations. Safe to call from many hosts at once: the
    /// migrator takes a database-level lock.
    pub async fn connect(url: &str) -> Result<Db, Error> {
        let db = Self::connect_without_migrating(url).await?;
        db.migrate().await?;
        Ok(db)
    }

    /// Connect without applying migrations (for short-lived helpers such as git hooks).
    pub async fn connect_without_migrating(url: &str) -> Result<Db, Error> {
        sqlx::any::install_default_drivers();
        let backend = Backend::from_url(url)?;
        let url = normalize_url(url, backend);
        if backend == Backend::Sqlite {
            // sqlx does not create the parent directory.
            if let Some(path) = url.strip_prefix("sqlite://").or_else(|| url.strip_prefix("sqlite:")) {
                let path = path.split('?').next().unwrap_or(path);
                if path != ":memory:"
                    && let Some(dir) = std::path::Path::new(path).parent()
                    && !dir.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(dir)?;
                }
            }
        }
        let max = match backend {
            Backend::Sqlite => 1, // one writer; sqlite serialises anyway and this avoids SQLITE_BUSY
            Backend::Postgres => 16,
        };
        let pool = AnyPoolOptions::new()
            .max_connections(max)
            .acquire_timeout(Duration::from_secs(30))
            .after_connect(move |conn: &mut AnyConnection, _meta| {
                Box::pin(async move {
                    if backend == Backend::Sqlite {
                        sqlx::raw_sql(
                            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 10000;",
                        )
                        .execute(&mut *conn)
                        .await?;
                    }
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .map_err(Error::internal)?;
        Ok(Db { pool, backend, url, health: Default::default() })
    }

    /// Apply pending migrations. sqlx holds an advisory lock (PostgreSQL) and runs each file
    /// in its own transaction (SQLite has a single writer), so concurrent starts are safe.
    /// The applied chain is checked first: a database whose applied migrations differ from this
    /// build's, or one migrated by a build newer than `SCHEMA_RANGE` allows, is refused with a
    /// clear message.
    pub async fn migrate(&self) -> Result<(), Error> {
        if self.check_applied_chain().await? == ChainState::ExpandedByNewerBuild {
            // Every embedded migration is applied and a newer build added more (expand and
            // contract): nothing to run, and sqlx would refuse the versions it does not know.
            return Ok(());
        }
        MIGRATOR.run(&self.pool).await.map_err(|e| match e {
            sqlx::migrate::MigrateError::VersionMismatch(v) => Error::msg(mismatch_message(v)),
            e => Error::internal(e),
        })
    }

    /// Compare `_sqlx_migrations` with the embedded set. Errors name the fix.
    async fn check_applied_chain(&self) -> Result<ChainState, Error> {
        use sqlx::migrate::Migrate;
        let mut c = self.pool.acquire().await.map_err(Error::internal)?;
        c.ensure_migrations_table(&MIGRATOR.table_name).await.map_err(Error::internal)?;
        let applied = c.list_applied_migrations(&MIGRATOR.table_name).await.map_err(Error::internal)?;
        drop(c);
        if applied.is_empty() {
            return Ok(ChainState::Fresh);
        }
        let (min, max) = (applied.iter().map(|a| a.version).min().unwrap_or(0), applied.iter().map(|a| a.version).max().unwrap_or(0));
        // A version we embed with different contents: the database was created by a build whose
        // migration files differ from this one's.
        for a in &applied {
            if let Some(m) = MIGRATOR.iter().find(|m| m.version == a.version)
                && m.checksum != a.checksum
            {
                return Err(Error::msg(mismatch_message(a.version)));
            }
        }
        if max > SCHEMA_RANGE.max {
            return Err(Error::msg(format!(
                "the database schema (migrations {min}..{max}) is newer than this build supports (up to {}); run a build that knows it or restore the database from before the upgrade",
                SCHEMA_RANGE.max
            )));
        }
        let unknown: Vec<i64> = applied.iter().map(|a| a.version).filter(|v| !MIGRATOR.iter().any(|m| m.version == *v)).collect();
        if unknown.is_empty() {
            return Ok(ChainState::Known);
        }
        let missing: Vec<i64> = MIGRATOR.iter().map(|m| m.version).filter(|v| !applied.iter().any(|a| a.version == *v)).collect();
        if !missing.is_empty() {
            return Err(Error::msg(format!(
                "the database was migrated by another build (versions {unknown:?}) but lacks migrations {missing:?} of this one; restore it or run the matching build's migrate"
            )));
        }
        tracing::info!(versions = ?unknown, "schema expanded by a newer build; running against it");
        Ok(ChainState::ExpandedByNewerBuild)
    }

    /// The highest applied migration version (0 when none).
    pub async fn schema_version(&self) -> Result<i64, Error> {
        use sqlx::migrate::Migrate;
        let mut c = self.pool.acquire().await.map_err(Error::internal)?;
        let applied = c.list_applied_migrations(&MIGRATOR.table_name).await.map_err(Error::internal)?;
        Ok(applied.iter().map(|m| m.version).max().unwrap_or(0))
    }

    /// Refuse to run against a schema outside `range` (see `SCHEMA_RANGE`).
    pub async fn check_schema(&self, range: SchemaRange) -> Result<i64, Error> {
        let v = self.schema_version().await?;
        if !range.contains(v) {
            return Err(Error::msg(format!(
                "database schema version {v} is outside the range this binary supports ({}..={}); run the matching release's migrate",
                range.min, range.max
            )));
        }
        Ok(v)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    pub async fn ping(&self) -> Result<(), Error> {
        let mut c = self.pool.acquire().await.map_err(Error::internal)?;
        c.ping().await.map_err(Error::internal)
    }

    /// Ping over the dedicated health pool rather than the application pool, so a busy or
    /// exhausted pool (SQLite has a single writer) does not make readiness checks wait, and
    /// probes do not open a new connection each time. The pool holds one connection, kept
    /// open (`min_connections = 1`); one that fails its ping is closed and the next probe
    /// connects afresh.
    pub async fn ping_dedicated(&self) -> Result<(), Error> {
        let pool = self
            .health
            .get_or_try_init(|| async {
                AnyPoolOptions::new()
                    .max_connections(1)
                    .min_connections(1)
                    .acquire_timeout(Duration::from_secs(5))
                    .test_before_acquire(false)
                    .connect(&self.url)
                    .await
                    .map_err(Error::internal)
            })
            .await?;
        let mut c = pool.acquire().await.map_err(Error::internal)?;
        match c.ping().await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = c.close().await;
                Err(Error::internal(e))
            }
        }
    }

    /// Connections the health pool holds (0 before the first probe), for tests.
    pub fn health_connections(&self) -> u32 {
        self.health.get().map(|p| p.size()).unwrap_or(0)
    }

    /// Case-insensitive LIKE that works on both engines. SQLite's LIKE is already
    /// case-insensitive for ASCII; on PostgreSQL we use ILIKE.
    pub fn ilike(&self) -> &'static str {
        match self.backend {
            Backend::Sqlite => "LIKE",
            Backend::Postgres => "ILIKE",
        }
    }
}

fn normalize_url(url: &str, backend: Backend) -> String {
    match backend {
        Backend::Sqlite => {
            // Accept "sqlite:path", "sqlite://path", and add mode=rwc so the file is created.
            let (base, query) = match url.split_once('?') {
                Some((b, q)) => (b.to_string(), Some(q.to_string())),
                None => (url.to_string(), None),
            };
            let mut q = query.unwrap_or_default();
            if !q.contains("mode=") {
                if !q.is_empty() {
                    q.push('&');
                }
                q.push_str("mode=rwc");
            }
            format!("{base}?{q}")
        }
        Backend::Postgres => url.to_string(),
    }
}

/// Map a sqlx error to the domain error, turning unique violations into conflicts.
pub(crate) fn map_err(e: sqlx::Error) -> Error {
    match &e {
        sqlx::Error::RowNotFound => Error::NotFound,
        sqlx::Error::Database(d) if d.is_unique_violation() => Error::Conflict("already exists".into()),
        _ => Error::internal(e),
    }
}

pub(crate) fn b(v: bool) -> i64 {
    if v { 1 } else { 0 }
}

pub(crate) fn now() -> i64 {
    reef_core::time::now_ms()
}
