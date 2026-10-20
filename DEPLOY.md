# Deploying Reef

How Reef runs on a host, behind a proxy, on several instances, in a container, and with
Actions runners. Configuration reference: `docs/configuration.md`; every key with its
default: `deploy/reef.conf.example`.

## 1. Files, unit, ports

The release tarball (`reef-<version>-linux-<arch>.tar.gz`) holds the static binary, the
systemd unit and the example configuration. Installed by hand it looks like this:

```
/usr/local/bin/reef                 the binary (static, no runtime dependencies beyond git, git-lfs, ssh-keygen)
/etc/reef/reef.conf                 configuration (TOML)
/etc/reef/conf.d/*.conf             operator drop-ins, merged after the main file in sorted order
/etc/reef/secrets/*                 0600, owned by reef (OIDC client secret, DB password)
/etc/reef/tls/{reef.crt,reef.key}   the service certificate, when Reef terminates TLS itself
/var/lib/reef/                      state: repositories/, hooks/, ssh/, actions/, lfs/, tmp/, reef.db (SQLite)
/var/log/reef/                      only when [log].file is set
/etc/systemd/system/reef.service    deploy/systemd/reef.service (also `reef print-unit`)
```

The unit runs as `reef:reef` with `Type=notify` and creates `/var/lib/reef` and
`/var/log/reef` itself (`StateDirectory`, `LogsDirectory`): Reef sends `READY=1` after
migrations are applied, the TLS certificate and key are parsed, and every listener (HTTP,
extra, Unix socket, SSH) is bound, `STOPPING=1` when it starts draining, and exits within
`TimeoutStopSec=30`.

Ports: `4920` HTTP(S) (git smart HTTP, API, UI, runners), `4922` SSH
(`ssh://git@host:4922/org/repo.git`), `4924` optional loopback metrics (`[metrics].listen`).

Binaries never write outside the state directory, `/tmp` (private) and the log directory;
large uploads and scratch files go under `/var/lib/reef/tmp`.

## 2. TLS and the proxy

Reef terminates TLS itself when `[tls]` is set (`cert_file`, `key_file`; rustls, TLS 1.2+).
Plain HTTP is for development only and is refused when `site_url` is `https://`. Each new
connection gets 10 s to finish its PROXY header and TLS handshake, and at most 1024 handshakes
per listener are in progress at once (further connections wait in the kernel backlog).

```toml
# TOML
listen        = ":4920"
site_url      = "https://reef.example.com"
ssh_listen    = ":4922"
data_dir      = "/var/lib/reef"
node_id       = 1
instance_name = "Reef"

[tls]
cert_file = "/etc/reef/tls/reef.crt"
key_file  = "/etc/reef/tls/reef.key"

[database]
url           = "postgres://reef@127.0.0.1:5432/reef?sslmode=disable"
password_file = "/etc/reef/secrets/db.password"

[auth]
mode = "oidc"

[oidc]
issuer             = "https://id.example.com"
client_id          = "reef"
client_secret_file = "/etc/reef/secrets/oidc-client.secret"
display_name       = "Example SSO"
```

Changing the provider's issuer URL (same provider, same subjects, new address): Reef keys
identities by (issuer, subject), so set the new `issuer` and keep the old URL in
`previous_issuers = ["https://old.example.com"]` (env `REEF_OIDC_PREVIOUS_ISSUERS`); each
user's next sign-in moves their identity to the new issuer. Or move everyone at once with
`reef rekey-issuer --from https://old.example.com --to https://id.example.com`. Without
either, existing users are asked to link their account on first sign-in. Only list former
URLs of the same provider. Details in `docs/configuration.md` ("Changing the issuer URL").

With a proxy in front (HAProxy, nginx, Caddy or a cloud load balancer on 80/443), either pass
TCP through to 4920 and let Reef terminate, or terminate at the proxy and re-encrypt to
Reef's own certificate. Do not downgrade to plain HTTP behind the proxy when `site_url` is
https: Reef then requires `[tls]`.

What `site_url` controls:

- Clone URLs, `html_url`/`url` fields in API responses, the Actions tenant URL handed to
  runners, and the OpenID Connect callback `<site_url>/login/oidc/callback`. Register that
  callback with the provider, and `<site_url>/` as a post-logout redirect URI.
- The back-channel logout endpoint `<site_url>/login/oidc/backchannel-logout`, which a
  provider that supports OpenID Connect Back-Channel Logout must be able to reach server to
  server.
- The session cookie is `HttpOnly; SameSite=Lax` and `Secure` when `site_url` is https. Reef
  does not read `X-Forwarded-Proto`; the scheme comes from configuration.

Proxy checklist:

| Requirement | Why |
| --- | --- |
| Forward `Host` unchanged | `api_url` is matched on the `Host` header |
| Idle timeout >= 60 s | Runner long-polls hold a request for up to 45 s; large pushes are slow |
| No sticky sessions | Sessions, tokens, device codes and runner sessions live in the database |
| Health check `GET /readyz` | 200 when the database answers; 503 while draining, so the instance leaves rotation before it stops |
| Request body limit >= largest push | Git over HTTPS streams pack files; Reef itself caps pushes at `[limits].max_push_bytes` (2 GiB) with 413 |
| Proxy listed in `[http] trusted_proxies` | So the client address (rate limits, request log, sessions) comes from `[http] forwarded_header` (last untrusted hop) or, with `[http] proxy_protocol`, the PROXY header, rather than being the proxy itself |
| TCP passthrough for 4922 (or 22) | Git over SSH is not HTTP; do not terminate TLS there |
| Port 80/443 for stock runners | `actions/runner` drops the port from `--url`; map 443 to 4920 on the proxy if runners register with a portless URL |

Reef logs one JSON line per request with the client address as `remote`. A proxy's
forwarding header (and its `X-Request-Id`) is believed only when the proxy is listed in
`[http] trusted_proxies`; the same client address is used for the request log, sessions and
rate limiting.

## 3. Start, migrate, upgrade

`reef serve` applies pending migrations at start, under sqlx's advisory lock on PostgreSQL
(each file in its own transaction on SQLite), so several instances can start at once. `reef
migrate` applies them without serving, for a controlled upgrade step; `config check` validates
the file and prints the effective configuration with secrets redacted.

Schema versions follow expand-and-contract: a migration may add tables, columns, indexes and
backfills, and may only drop or rename what the previous release no longer uses, so binary N
runs against schema N+1. Each build declares the schema range it supports (`SCHEMA_RANGE` in
`reef-db`: its own latest migration and the next one); `serve` refuses to start against a
schema outside it, naming both. Upgrade order: new binaries may migrate on start (the old
ones keep running against the expanded schema); roll back by reinstalling the previous
release without touching the database.

The first account is the site administrator: in standalone mode the first sign-up, in oidc
mode the first provider sign-in. `reef set-admin LOGIN` grants it later, and in oidc mode a
boolean claim named by `[oidc] admin_claim` grants it at sign-in.

## 4. Several instances behind one load balancer

Every Reef process is stateless apart from two shared resources:

1. **PostgreSQL.** `[database].url = postgres://...` on every instance. SQLite cannot be
   shared between hosts. Accounts, repository metadata, issues, projects, sessions, runner
   registrations and the job queue live here. Job claims are single atomic
   `UPDATE ... WHERE status = 'queued'` statements (with `FOR UPDATE SKIP LOCKED` on
   PostgreSQL), so several instances running the scheduler do not double-assign work.
2. **A shared `data_dir`.** It holds the bare git repositories and wikis (`repositories/`),
   the server-side git hooks (`hooks/`), the SSH host key (`ssh/`), the runner JWT signing key
   and secret keys (`actions/keys/`), Actions job logs and artifacts (`actions/`) and LFS
   objects (`lfs/`). Sharing it means every instance serves any repository, presents the same
   SSH host key, and validates runner and job tokens minted by a sibling. The keys are created
   once with `O_EXCL`; a second instance that finds them uses them and never regenerates them.

Filesystem choices:

- **CephFS** works as is: POSIX rename, `O_EXCL` lock files and fsync behave as git expects.
- **NFS** works with NFSv4, `hard` mounts, locking enabled and without aggressive attribute
  caching (`actimeo=1` or `noac` on write-heavy hosts). Avoid NFSv3 without a lock manager.
- **Not object storage.** Git needs a real filesystem. An object-store backend for logs,
  artifacts and LFS is on the backlog (`TODO.md`).

Outbound connections (webhooks, mirrors, imports) are made on behalf of repository admins. By
default targets that resolve to loopback, link-local, private or CGNAT ranges are refused
(`[network] allow_private_targets = false`). Only enable it on isolated networks.

Per instance:

- A unique `node_id` (1..15). It is embedded in every generated id, so instances never
  collide on ids without coordination.
- Identical `site_url`, `[database]`, `data_dir`, `[oidc]` and `[actions]` settings.

State that is intentionally per instance (caches that rebuild themselves): repository
statistics and contribution calendars, and the API rate-limit buckets (each instance
enforces `[limits]` on the traffic it sees; see `docs/configuration.md`, Rate limits). A
back-channel logout may reach any instance: sessions live in the database, so it ends them
everywhere.

Background work runs on every instance and is coordinated through the database, so no
instance is special and losing one loses no work:

| Loop | Coordination |
| --- | --- |
| Actions job dispatch (runner long-poll) | one guarded `UPDATE ... WHERE status = 'queued'`; `FOR UPDATE SKIP LOCKED` on PostgreSQL; runners renew a lock while a job runs and a job whose lock expires is requeued |
| Webhook poller (30 s) | transactional outbox: each event is queued in the transaction that records it; entries are claimed under a 60 s lease (`FOR UPDATE SKIP LOCKED` on PostgreSQL) and deleted in the transaction that records their deliveries, only while the claim holds, so a sibling whose lease was taken over commits nothing and a late-committing event is never skipped |
| Webhook dispatcher (30 s) | rows claimed under a 60 s lease (`FOR UPDATE SKIP LOCKED` on PostgreSQL, guarded update on SQLite), renewed before each post; an expired lease is claimable again |
| Pull and push mirrors (30 s) | a 5 min lease per mirror in `mirror_config` / `push_mirrors`, renewed every minute while git runs and released with the outcome, both compare-and-set on the lease value: an instance whose lease was taken over aborts its git process and records nothing; two instances never sync the same mirror |
| Project workflows (30 s) | idempotent updates |
| Retention (hourly) | idempotent; files are removed before rows, so passes on several instances share the work |
| Metrics gauges (30 s) | read-only |

Put the proxy's addresses in `[http] trusted_proxies` so rate limiting sees client
addresses rather than the proxy's.

```
clients  --https-->  LB --tcp:4920-->  reef-1 --+
          --tcp:22-->    --tcp:4922-->  reef-2 --+--> PostgreSQL
runners  --https-->  LB                          +--> CephFS / NFS   /var/lib/reef
```

## 5. Container

Images for linux/amd64 and linux/arm64 are published at `yadvlabs/reef` on Docker Hub,
tagged with the release version (`0.1.0`, `0.1`) and `latest`. The image runs as `reef`
(uid 4920) with `git`, `git-lfs` and `ssh-keygen` installed; the binary is the same static
one as in the release tarball.

```sh
docker run -d --name reef -p 4920:4920 -p 4922:4922 \
  -v reef-data:/var/lib/reef \
  -e REEF_SITE_URL=https://reef.example.com \
  yadvlabs/reef:latest
```

- Configuration: `REEF_*` variables, or mount a TOML file at `/etc/reef/reef.conf` (and
  drop-ins under `/etc/reef/conf.d/`). With nothing set Reef runs in standalone mode on
  SQLite and the first sign-up is the site administrator.
- State: a volume at `/var/lib/reef`. A bind mount must be writable by uid 4920.
- TLS: terminate at a proxy in front and keep `site_url` https; or mount the certificate and
  set `REEF_TLS_CERT_FILE` / `REEF_TLS_KEY_FILE`.
- Readiness: `GET /readyz` on 4920 (200 when the database answers, 503 while draining).
- `deploy/docker-compose.yml` runs the same with an optional PostgreSQL service.

`deploy/Dockerfile` builds the web UI, cross-compiles the binary for the target architecture
with cargo-zigbuild on the build host (no emulation), and copies it into a slim Debian
runtime. `make image` builds it for the current machine; the release workflow builds and
pushes both architectures.

## 6. Actions and runners

Workflows live in `.github/workflows/*.yml` in each repository and use the GitHub workflow
syntax. Reef runs them on stock `actions/runner` binaries; there is no Reef-specific runner.
Nothing runs a job until a runner is registered for the repository or its organization;
queued jobs wait until a runner whose labels match `runs-on` picks them up.

Register a runner:

1. Download the runner for the machine's platform (`gh api /repos/{o}/{r}/actions/runners/downloads`).
2. Mint a registration token (repository admins, or org owners for org runners):

   ```sh
   gh api -X POST /repos/acme/app/actions/runners/registration-token   # repo runner
   gh api -X POST /orgs/acme/actions/runners/registration-token        # org runner
   ```

3. Configure and start it:

   ```sh
   ./config.sh --unattended --url https://reef.example.com/acme/app \
               --token <registration token> --name build-01 --labels linux,x64,docker
   ./run.sh                                       # or: sudo ./svc.sh install && sudo ./svc.sh start
   ```

   Use `--url https://reef.example.com/acme` for an organization runner, `--ephemeral` for
   one-job runners, `--replace` to re-register under an existing name. The runner keeps only
   the host of `--url`, so it must reach Reef on 443 (or 80): map it on the proxy.

The runner talks to `<site_url>/_actions/{repo|org}/{id}` (see `docs/02-runner-protocol.md`).
Reef fetches actions that are not hosted locally from `[actions].upstream` (github.com by
default) and proxies them to the runner, so the runner host does not need github.com access.

Labels: runners register with `self-hosted`, their OS and architecture, plus the labels you
pass; `runs-on: ubuntu-latest` only matches a runner carrying that label. Secrets and
variables: `gh secret set` / `gh variable set`, or `Settings -> Secrets and variables`. Every
job receives `GITHUB_TOKEN` (a Reef job token scoped to the repository) and the usual
`GITHUB_*` environment. Logs and artifacts live under `data_dir/actions`; completed runs
older than `[actions].retention` (default `"90d"`; the site console can override it; `0`
keeps everything) are removed with their logs and artifacts by an hourly retention pass on
every instance, which also drops expired artifacts and scratch files under `data_dir/tmp`
older than a day. `reef_actions_logs_bytes` and `reef_actions_artifacts_bytes` on `/metrics`
show what the tree holds (`docs/metrics.md`).

Not available yet: artifact upload (`actions/upload-artifact`), the cache service
(`actions/cache`), reusable workflows (`workflow_call`) and `schedule` triggers (`TODO.md`).
