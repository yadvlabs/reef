//! The `reef` binary: `serve`, `migrate`, `config check`, `version`, `print-unit`, the
//! account commands, `rekey-issuer`, and the git hook entry point.

mod logging;

use clap::{Parser, Subcommand};
use reef_api::config::LoadOptions;
use reef_api::{AppState, Config, Listeners};
use reef_db::Db;
use reef_db::accounts::NewUser;
use reef_git::Store;
use std::path::PathBuf;
use std::time::Duration;

/// `v<version> (<commit>)`, shared with the API's meta endpoint.
const VERSION: &str = reef_api::VERSION;
/// The unit shipped in the bundle; `print-unit` prints exactly this.
const UNIT: &str = include_str!("../../../deploy/systemd/reef.service");
/// The annotated configuration reference; `config example` prints exactly this.
const EXAMPLE: &str = include_str!("../../../deploy/reef.conf.example");
/// After a stop signal `/readyz` answers 503 while the listeners keep accepting for this
/// long, so load balancers take the instance out of rotation before connections are refused.
const READINESS_DRAIN: Duration = Duration::from_secs(5);
/// Then the listeners stop and in-flight requests and SSH sessions get this long to finish.
/// Both together stay under the unit's `TimeoutStopSec=30`.
const DRAIN: Duration = Duration::from_secs(20);

#[derive(Parser)]
// `--version` prints the same line as `version`: "reef <version> (<commit>)".
#[command(name = "reef", display_name = "reef", version = VERSION, about = "Reef: GitHub-compatible source hosting")]
struct Cli {
    /// Configuration file (TOML). Default /etc/reef/reef.conf, which may be absent; a
    /// path given here must exist. `conf.d/*.conf` next to it and REEF_* variables override it.
    #[arg(short, long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Apply migrations, then serve HTTP(S), SSH and the health endpoints until SIGTERM.
    Serve {
        /// Primary listener, e.g. `127.0.0.1:4920` (overrides `listen`).
        #[arg(long, value_name = "ADDR")]
        listen: Option<String>,
    },
    /// Apply pending database migrations and exit.
    Migrate,
    /// Configuration commands.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Print "reef <version> (<commit>)".
    Version,
    /// Print the systemd unit shipped with this build.
    PrintUnit,
    /// Create a user (standalone mode).
    CreateUser {
        login: String,
        email: String,
        /// The password; REEF_PASSWORD keeps it off the command line.
        #[arg(long, env = "REEF_PASSWORD", hide_env_values = true)]
        password: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        admin: bool,
    },
    /// Grant or revoke site administrator: `set-admin LOGIN` grants, `set-admin LOGIN --admin false` revokes.
    SetAdmin {
        login: String,
        #[arg(long, action = clap::ArgAction::Set, default_value_t = true, value_name = "true|false")]
        admin: bool,
    },
    /// Reset a user's password and end their sessions.
    SetPassword {
        login: String,
        #[arg(long, env = "REEF_PASSWORD", hide_env_values = true)]
        password: String,
    },
    /// Create an organization owned by a user.
    CreateOrg {
        login: String,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Mint a personal access token for a user and print it.
    CreateToken {
        login: String,
        #[arg(long, default_value = "cli")]
        name: String,
        #[arg(long, default_value = "repo,read:org,workflow,admin:public_key,delete_repo,user")]
        scopes: String,
    },
    /// Move every OIDC identity recorded under a former issuer URL to the current
    /// one (the same provider, re-homed to a new URL; subjects unchanged). Identities whose
    /// subject already exists under `--to` are left as they are.
    RekeyIssuer {
        /// The former issuer URL, as recorded (with and without a trailing slash is tried).
        #[arg(long, value_name = "URL")]
        from: String,
        /// The current issuer URL, exactly as the provider's ID tokens carry it (`iss`).
        #[arg(long, value_name = "URL")]
        to: String,
    },
    /// Git hook entry point (invoked by git, not by people).
    #[command(hide = true)]
    Hook { name: String },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Load and validate the configuration, print it with secrets redacted; exit 1 on error.
    Check,
    /// Print the annotated configuration reference (every key with its default), e.g.
    /// `reef config example > /etc/reef/reef.conf`.
    Example,
}

async fn open(cfg: &Config) -> anyhow::Result<(Db, Store)> {
    reef_core::id::install(cfg.node_id);
    std::fs::create_dir_all(&cfg.data_dir).map_err(|e| anyhow::anyhow!("data_dir {}: {e}", cfg.data_dir.display()))?;
    std::fs::create_dir_all(cfg.tmp_dir())?;
    let db = Db::connect(&cfg.database_url()).await?;
    let hooks = reef_git::hooks::install(&cfg.data_dir)?;
    let store = Store::new(cfg.repos_dir(), &cfg.git_binary).with_hooks(hooks).with_tmp_dir(cfg.tmp_dir());
    std::fs::create_dir_all(store.root())?;
    Ok((db, store))
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("reef: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    reef_api::install_crypto_provider();
    match &cli.cmd {
        Cmd::Version => {
            println!("reef {VERSION}");
            return Ok(());
        }
        Cmd::PrintUnit => {
            print!("{UNIT}");
            return Ok(());
        }
        Cmd::Config { cmd: ConfigCmd::Example } => {
            print!("{EXAMPLE}");
            return Ok(());
        }
        Cmd::Hook { name } => {
            logging::init_plain("warn");
            return hooks::run(name).await;
        }
        _ => {}
    }
    let listen = match &cli.cmd {
        Cmd::Serve { listen } => listen.clone(),
        _ => None,
    };
    let (cfg, sources) = Config::load(&LoadOptions { path: cli.config.clone(), listen })?;
    if let Cmd::Config { cmd: ConfigCmd::Check } = &cli.cmd {
        print!("{}", cfg.render_redacted(&sources));
        return Ok(());
    }
    let _log_guard = logging::init(&cfg.log)?;

    match cli.cmd {
        Cmd::Serve { .. } => serve(cfg).await,
        Cmd::Migrate => {
            let (db, _) = open(&cfg).await?;
            let v = db.schema_version().await?;
            println!("migrations applied ({}); schema version {v}", db.backend().name());
            Ok(())
        }
        Cmd::CreateUser { login, email, password, name, admin } => {
            let (db, _) = open(&cfg).await?;
            let u = db
                .create_user(NewUser { login: &login, email: &email, password: Some(&password), name: name.as_deref(), is_admin: admin })
                .await?;
            println!("created user {} (id {}){}", u.login, u.id, if admin { ", site admin" } else { "" });
            Ok(())
        }
        Cmd::SetAdmin { login, admin } => {
            let (db, _) = open(&cfg).await?;
            let u = db.account_by_login(&login).await?;
            db.set_admin(u.id(), admin).await?;
            println!("{} is {}a site admin", u.login, if admin { "" } else { "no longer " });
            Ok(())
        }
        Cmd::SetPassword { login, password } => {
            let (db, _) = open(&cfg).await?;
            let u = db.account_by_login(&login).await?;
            db.set_password(u.id(), &password).await?;
            db.delete_user_sessions(u.id()).await?;
            println!("password updated for {}", u.login);
            Ok(())
        }
        Cmd::CreateOrg { login, owner, name } => {
            let (db, _) = open(&cfg).await?;
            let o = db.account_by_login(&owner).await?;
            let org = db.create_org(&login, o.id(), name.as_deref(), None).await?;
            println!("created organization {} (id {}) owned by {}", org.login, org.id, o.login);
            Ok(())
        }
        Cmd::CreateToken { login, name, scopes } => {
            let (db, _) = open(&cfg).await?;
            let u = db.account_by_login(&login).await?;
            let scopes: Vec<&str> = scopes.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
            let (plain, _) = db.create_token(u.id(), "pat", &name, &scopes, None, None).await?;
            println!("{plain}");
            Ok(())
        }
        Cmd::RekeyIssuer { from, to } => {
            for u in [&from, &to] {
                if !u.starts_with("http://") && !u.starts_with("https://") {
                    anyhow::bail!("issuer must be a URL, got {u:?}");
                }
            }
            let base = from.trim_end_matches('/');
            if base == to.trim_end_matches('/') {
                anyhow::bail!("--from and --to name the same issuer");
            }
            let (db, _) = open(&cfg).await?;
            let (mut moved, mut left) = (0, 0);
            for f in [base.to_string(), format!("{base}/")] {
                let (m, l) = db.rekey_issuer(&f, &to).await?;
                moved += m;
                left += l;
            }
            tracing::info!(from = %base, to = %to, moved, left, "re-keyed provider identities to a new issuer");
            println!("moved {moved} identities from {base} to {to}");
            if left > 0 {
                println!("{left} left under {base}: their subject is already linked under {to}");
            }
            Ok(())
        }
        Cmd::Version | Cmd::PrintUnit | Cmd::Hook { .. } | Cmd::Config { .. } => unreachable!("handled above"),
    }
}

/// Migrate, bind every listener, announce readiness, serve until a stop signal, then drain.
async fn serve(cfg: Config) -> anyhow::Result<()> {
    let (db, store) = open(&cfg).await?;
    let schema = db.check_schema(reef_db::SCHEMA_RANGE).await?;
    tracing::info!(
        version = VERSION,
        listen = %cfg.listen_addr(),
        site = %cfg.site_url(),
        api = %cfg.api_url(),
        mode = %cfg.auth.mode,
        tls = cfg.tls.enabled(),
        db = db.backend().name(),
        schema,
        data_dir = %cfg.data_dir.display(),
        "reef starting"
    );
    for w in cfg.warnings() {
        tracing::warn!("{w}");
    }
    let state = AppState::new(db, store, cfg.clone())?;
    // TLS material is parsed before anything is bound or announced: a bad certificate or key
    // fails the start, not the first connection after READY=1.
    let tls = reef_api::tls_config(&cfg).await?;
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                st.limiter.sweep();
                reef_api::actions::tick(&st).await;
                reef_api::projects::run_scheduled_workflows(&st).await;
                reef_api::webhooks::tick(&st).await;
                reef_api::mirror::tick(&st).await;
            }
        });
        // Product gauges on /metrics (every 30 s) and the Actions retention job (hourly).
        let st3 = state.clone();
        tokio::spawn(async move { reef_api::metrics::run(st3).await });
        let st4 = state.clone();
        tokio::spawn(async move { reef_api::actions::retention::run(st4).await });
    }

    // Bind everything before READY=1.
    let bind = |addr: std::net::SocketAddr, what: &'static str| async move {
        tokio::net::TcpListener::bind(addr).await.map_err(|e| anyhow::anyhow!("{what} {addr}: {e}"))
    };
    let main = bind(cfg.listen_addr(), "listen").await?;
    let mut extra = Vec::new();
    for addr in cfg.listen_extra_addrs() {
        extra.push(bind(addr, "listen_extra").await?);
        tracing::info!(%addr, "listening (extra)");
    }
    let unix = match cfg.listen_unix_path() {
        Some(p) => {
            let _ = std::fs::remove_file(&p);
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            tracing::info!(path = %p.display(), "listening on unix socket");
            Some(tokio::net::UnixListener::bind(&p).map_err(|e| anyhow::anyhow!("listen_unix {}: {e}", p.display()))?)
        }
        None => None,
    };
    let metrics = match cfg.metrics_listen_addr() {
        Some(addr) => {
            if !addr.ip().is_loopback() {
                // Accepted, but /metrics is then reachable from the network.
                tracing::warn!(%addr, "metrics.listen is not a loopback address; /metrics is served to the network there");
            }
            tracing::info!(%addr, "metrics listener");
            Some(bind(addr, "metrics.listen").await?)
        }
        None => None,
    };
    let ssh = match cfg.ssh_listen_addr() {
        Some(addr) => Some(bind(addr, "ssh_listen").await?),
        None => None,
    };

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let on_stop = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    };
    // The SSH server runs as its own task; if it ends before shutdown (or with an error) that is
    // logged at once and `/readyz` reports `ssh` as failed, since git-over-SSH is part of the
    // service. The watcher hands the outcome on so the drain can still await it.
    let ssh_task = ssh.map(|listener| {
        let server = reef_ssh::SshServer::new(state.clone());
        let data_dir = cfg.data_dir.clone();
        let stop = on_stop(stop_rx.clone());
        let st = state.clone();
        let inner = tokio::spawn(async move { server.run(listener, &data_dir, stop).await });
        tokio::spawn(async move {
            let outcome = inner.await;
            let stopping = st.shutting_down.load(std::sync::atomic::Ordering::Relaxed);
            match &outcome {
                Ok(Ok(())) if stopping => {}
                Ok(Ok(())) => {
                    st.mark_ssh_failed();
                    tracing::error!("ssh server stopped unexpectedly; /readyz now reports ssh as failed");
                }
                Ok(Err(e)) => {
                    st.mark_ssh_failed();
                    tracing::error!(error = %e, "ssh server failed; /readyz now reports ssh as failed");
                }
                Err(e) => {
                    st.mark_ssh_failed();
                    tracing::error!(error = %e, "ssh server task panicked; /readyz now reports ssh as failed");
                }
            }
            outcome.map_err(|e| reef_core::Error::msg(e.to_string())).and_then(|r| r)
        })
    });
    let mut http_task =
        tokio::spawn(reef_api::serve(Listeners { main, extra, unix, metrics }, tls, state.clone(), on_stop(stop_rx.clone())));

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready, sd_notify::NotifyState::Status("serving")]);
    tracing::info!("ready");

    tokio::select! {
        _ = stop_signal() => {}
        r = &mut http_task => {
            return Err(match r {
                Ok(Ok(())) => anyhow::anyhow!("http server stopped unexpectedly"),
                Ok(Err(e)) => anyhow::anyhow!("http server failed: {e}"),
                Err(e) => anyhow::anyhow!("http server task failed: {e}"),
            });
        }
    }

    // 1. Readiness drain: /readyz is 503, everything else keeps being served.
    tracing::info!(readiness_drain_secs = READINESS_DRAIN.as_secs(), drain_secs = DRAIN.as_secs(), "shutting down");
    state.begin_shutdown();
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping, sd_notify::NotifyState::Status("draining")]);
    tokio::select! {
        _ = tokio::time::sleep(READINESS_DRAIN) => {}
        _ = &mut http_task => { tracing::warn!("http server ended during the readiness drain"); }
    }
    // 2. Stop the listeners and let in-flight requests and SSH sessions finish.
    let _ = stop_tx.send(true);
    let drain = async {
        if !http_task.is_finished() {
            match http_task.await {
                Ok(Err(e)) => tracing::warn!(error = %e, "http server ended with an error"),
                Err(e) => tracing::warn!(error = %e, "http server task failed"),
                Ok(Ok(())) => {}
            }
        }
        if let Some(t) = ssh_task {
            match t.await {
                Ok(Err(e)) => tracing::warn!(error = %e, "ssh server ended with an error"),
                Err(e) => tracing::warn!(error = %e, "ssh watcher task failed"),
                Ok(Ok(())) => {}
            }
        }
    };
    if tokio::time::timeout(DRAIN, drain).await.is_err() {
        tracing::warn!("drain deadline reached; exiting with connections open");
    }
    tracing::info!("stopped");
    Ok(())
}

async fn stop_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };
    #[cfg(unix)]
    let term = async { tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok()?.recv().await };
    #[cfg(not(unix))]
    let term = std::future::pending::<Option<()>>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

mod hooks {
    //! `reef hook pre-receive`: enforce branch protection. Runs inside `git receive-pack`
    //! with the environment set by the HTTP or SSH handler (`REEF_DATABASE_URL`, `REEF_REPO_ID`,
    //! `REEF_PUSHER_*`); nothing is read from the configuration file.
    use reef_core::Id;
    use reef_db::Db;
    use std::io::{BufRead, Write};

    pub async fn run(name: &str) -> anyhow::Result<()> {
        if name != "pre-receive" {
            return Ok(());
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let (Some(db_url), Some(repo_id)) = (env("REEF_DATABASE_URL"), env("REEF_REPO_ID")) else {
            // Not spawned by Reef: allow, so plain local pushes into the store still work.
            return Ok(());
        };
        let repo_id = Id(repo_id.parse()?);
        let is_admin = env("REEF_PUSHER_ADMIN").as_deref() == Some("1");
        let internal = env("REEF_INTERNAL_PUSH").is_some();
        // The server passes its configured git binary along; a hook run by hand uses PATH.
        let git = env("REEF_GIT_BINARY").unwrap_or_else(|| "git".into());
        let repo_dir = std::env::var("GIT_DIR").unwrap_or_else(|_| ".".into());

        let mut updates = Vec::new();
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            let mut parts = line.split(' ');
            if let (Some(old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next()) {
                updates.push((old.to_string(), new.to_string(), name.to_string()));
            }
        }
        let protected: Vec<_> = updates.iter().filter(|(_, _, n)| n.starts_with("refs/heads/")).collect();
        if protected.is_empty() || internal {
            return Ok(());
        }
        let db = Db::connect_without_migrating(&db_url).await?;
        let rules = db.protections(repo_id).await?;
        if rules.is_empty() {
            return Ok(());
        }
        let zero = "0000000000000000000000000000000000000000";
        let mut stderr = std::io::stderr();
        for (old, new, name) in protected {
            let branch = &name["refs/heads/".len()..];
            let Some(rule) = rules.iter().find(|r| r.matches(branch)) else { continue };
            if is_admin && rule.enforce_admins == 0 {
                continue;
            }
            let mut reject = |msg: &str| -> anyhow::Result<()> {
                writeln!(stderr, "reef: refusing to update {name}: {msg}")?;
                writeln!(stderr, "reef: branch '{branch}' is protected by the rule '{}'", rule.pattern)?;
                std::process::exit(1);
            };
            if rule.lock_branch != 0 {
                reject("the branch is locked (read-only)")?;
            }
            if new == zero {
                if rule.allow_deletions == 0 {
                    reject("deleting this branch is not allowed")?;
                }
                continue;
            }
            if rule.require_pull_request != 0 {
                reject("changes must be made through a pull request")?;
            }
            if old != zero && rule.allow_force_pushes == 0 {
                let ff = reef_git::hooks::is_ancestor(std::path::Path::new(&git), std::path::Path::new(&repo_dir), old, new).await?;
                if !ff {
                    reject("force pushes are not allowed")?;
                }
            }
            // For a new branch this judges what it adds over the default branch (or its whole
            // history when there is none), so a merge commit on a new protected branch is caught.
            if rule.require_linear_history != 0
                && reef_git::hooks::introduces_merge_commits(std::path::Path::new(&git), std::path::Path::new(&repo_dir), old, new).await?
            {
                reject("merge commits are not allowed (linear history required)")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// `version` and `--version` print the same `reef <version> (<commit>)` line.
    #[test]
    fn version_lines_match() {
        let line = Cli::command().render_version().to_string();
        assert_eq!(line, format!("reef {VERSION}\n"));
        assert!(VERSION.starts_with('v') && VERSION.contains(" (") && VERSION.ends_with(')'), "{VERSION}");
    }

    #[test]
    fn the_unit_is_hardened() {
        assert!(UNIT.contains("\nUMask=0077\n"));
        assert!(UNIT.contains("\nRequiresMountsFor=/var/lib/reef\n"));
        assert!(UNIT.contains("\nTimeoutStopSec=30\n"));
    }
}
