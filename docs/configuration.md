# Configuration and operations

The binary is `reef`; it reads `/etc/reef/reef.conf` (TOML, `# TOML` first line) unless
`--config PATH` is given, then `conf.d/*.conf` next to that file in sorted order, then `REEF_*`
environment variables, then the `--listen` flag. A missing default file is fine when the
defaults plus the environment are enough (containers, development); a missing `--config` path
is an error. Unknown keys are an error naming the file and key. `deploy/reef.conf.example`
lists every key with its default.

```
reef serve   [--config PATH] [--listen ADDR]   migrate at start, then serve until SIGTERM
reef migrate [--config PATH]                   apply migrations and exit
reef config check [--config PATH]              load, validate, print the effective config (secrets redacted) and warnings
reef config example                            print the annotated configuration reference (every key, its default)
reef version | --version                       "reef v0.1.1 (<commit>)"
reef print-unit                                the systemd unit shipped with this build
reef create-user | set-admin | set-password | create-org | create-token | rekey-issuer
```

Environment twins: `REEF_<KEY>` for top-level keys, `REEF_<SECTION>_<KEY>` for sections.
Arrays are comma-separated; booleans are `true|false|1|0`. Secrets are never inline: each
secret key is a `*_file` path, and its env twin without `_FILE` carries the value directly for
containers and development.

## Keys

| Key | Env | Default | Meaning |
| --- | --- | --- | --- |
| `listen` | `REEF_LISTEN` | `":4920"` | Primary listener; HTTPS when `[tls]` is set, else HTTP. `:port` = every interface. |
| `site_url` | `REEF_SITE_URL` | `"http://localhost:4920"` | Public URL: clone URLs, API URLs, the OIDC callback, cookie `Secure`. |
| `ssh_listen` | `REEF_SSH_LISTEN` | `":4922"` | Git-over-SSH listener; `""` disables SSH. |
| `ssh_host` | `REEF_SSH_HOST` | `""` | Host in `ssh://` clone URLs; `""` = host of `site_url`. |
| `ssh_port` | `REEF_SSH_PORT` | `0` | Port in `ssh://` clone URLs; `0` = port of `ssh_listen`. |
| `api_url` | `REEF_API_URL` | `""` | REST root; `""` = `<site_url>/api/v3`. An `api.` host mirrors api.github.com. |
| `listen_extra` | `REEF_LISTEN_EXTRA` | `[]` | Additional TCP listeners for the same application. |
| `listen_unix` | `REEF_LISTEN_UNIX` | `""` | Unix socket to serve on as well (reverse proxies, local `gh`). |
| `data_dir` | `REEF_DATA_DIR` | `"/var/lib/reef"` | State: `repositories/`, `hooks/`, `ssh/`, `actions/`, `lfs/`, `tmp/`, `reef.db`. |
| `node_id` | `REEF_NODE_ID` | `1` | 1..15, unique per host; embedded in every generated id. Anything else is an error. |
| `instance_name` | `REEF_INSTANCE_NAME` | `"Reef"` | Name shown in the UI. |
| `git_binary` | `REEF_GIT_BINARY` | `"git"` | The git executable. |
| `allow_signup` | `REEF_ALLOW_SIGNUP` | `false` | Self-service accounts (standalone mode only; the first account can always be created). |
| `tls.cert_file` | `REEF_TLS_CERT_FILE` | `""` | PEM chain. Both TLS files empty = plain HTTP: development, or behind a proxy that terminates TLS (`site_url` https, proxy in `http.trusted_proxies`). |
| `tls.key_file` | `REEF_TLS_KEY_FILE` | `""` | PEM key. |
| `tls.client_ca_file` | `REEF_TLS_CLIENT_CA_FILE` | `""` | Reserved for mTLS. |
| `database.url` | `REEF_DATABASE_URL` | `""` | `postgres://user@host:5432/reef?sslmode=disable` or `sqlite:///path/reef.db`; `""` = `sqlite://<data_dir>/reef.db`. |
| `database.password_file` | `REEF_DATABASE_PASSWORD_FILE` | `""` | Inserted into the URL. Value twin: `REEF_DATABASE_PASSWORD`. |
| `log.format` | `REEF_LOG_FORMAT` | `"json"` | `json` or `text` (`make dev` uses text). |
| `log.level` | `REEF_LOG_LEVEL` | `"info"` | `error`, `warn`, `info`, `debug` or `trace`; `RUST_LOG` overrides. |
| `log.file` | `REEF_LOG_FILE` | `""` | Daily rolling file, 14 kept, in addition to stderr. |
| `metrics.listen` | `REEF_METRICS_LISTEN` | `""` | When set (e.g. `127.0.0.1:4924`): `/metrics` is served only there and the primary listener answers 404 for it; `/healthz` and `/readyz` stay on the primary listener. |
| `http.trusted_proxies` | `REEF_HTTP_TRUSTED_PROXIES` | `[]` | Reverse proxies (addresses or CIDRs) whose forwarding header is believed for the client address (rate limits, request log `remote`, sessions) and whose `X-Request-Id` is kept. Empty: the socket peer is the client and the header is ignored. |
| `http.forwarded_header` | `REEF_HTTP_FORWARDED_HEADER` | `"x-forwarded-for"` | The header trusted proxies set: `x-forwarded-for` or `forwarded` (RFC 7239 `for=`). |
| `http.proxy_protocol` | `REEF_HTTP_PROXY_PROTOCOL` | `false` | Accept PROXY protocol v1/v2 on the HTTP(S) TCP listeners only; the SSH listener never accepts it. Optional for trusted proxies (their header sets the peer; none is fine, e.g. health probes); a header from an untrusted peer drops the connection; untrusted peers without one are served as usual. |
| `auth.mode` | `REEF_AUTH_MODE` | `"standalone"` | `standalone` (built-in accounts) refuses an `[oidc]` section; `oidc` needs one. |
| `oidc.issuer` | `REEF_OIDC_ISSUER` | `""` | Issuer URL, e.g. `https://id.example.com`; discovery at `<issuer>/.well-known/openid-configuration`. Any OpenID Connect provider works. |
| `oidc.client_id` | `REEF_OIDC_CLIENT_ID` | `"reef"` | Registered client id. |
| `oidc.client_secret_file` | `REEF_OIDC_CLIENT_SECRET_FILE` | `""` | Required in oidc mode. Value twin: `REEF_OIDC_CLIENT_SECRET`. |
| `oidc.ca_file` | `REEF_OIDC_CA_FILE` | `""` | PEM bundle trusted for the issuer; `""` = system roots. |
| `oidc.scopes` | `REEF_OIDC_SCOPES` | `["openid", "profile", "email"]` | Scopes requested at sign-in. |
| `oidc.link_by_email` | `REEF_OIDC_LINK_BY_EMAIL` | `false` | Link unknown subjects to an account with the same verified email. |
| `oidc.display_name` | `REEF_OIDC_DISPLAY_NAME` | `"SSO"` | Provider name on the sign-in button ("Sign in with ...") and in messages. |
| `oidc.admin_claim` | `REEF_OIDC_ADMIN_CLAIM` | `""` | A boolean ID token claim that makes the person a site administrator when `true`; `""` = none. |
| `oidc.previous_issuers` | `REEF_OIDC_PREVIOUS_ISSUERS` | `[]` | Former issuer URLs of the same provider (comma-separated in the env twin). An identity recorded under one of them is moved to `issuer` on its next sign-in. See [Changing the issuer URL](#changing-the-issuer-url). |
| `actions.upstream` | `REEF_ACTIONS_UPSTREAM` | `"https://codeload.github.com"` | Where actions not hosted here are fetched from (`<upstream>/{owner}/{repo}/tar.gz/{ref}`). |
| `actions.runner_download_base` | `REEF_ACTIONS_RUNNER_DOWNLOAD_BASE` | GitHub releases | Base URL of the runner download list. |
| `actions.retention` | `REEF_ACTIONS_RETENTION` | `"90d"` | How long after completing a workflow run is removed with its logs and artifacts, as a duration (`"90d"`, `"12h"`); `"0"` keeps runs forever; clamped to 100 years. The site console's `actions_retention_days` (whole days) overrides it. |
| `lfs.enabled` | `REEF_LFS_ENABLED` | `true` | Serve the Git LFS batch and transfer API under `<data_dir>/lfs`. |
| `lfs.max_object_size` | `REEF_LFS_MAX_OBJECT_SIZE` | `5368709120` | Uploads above this are refused with 507. |
| `mirror.allow_file_urls` | `REEF_MIRROR_ALLOW_FILE_URLS` | `false` | `file://` sources for mirrors and imports (development only). |
| `mirror.timeout` | `REEF_MIRROR_TIMEOUT` | `"1h"` | Time limit for a mirror fetch, push or import, as a duration (`"1h"`, `"30m"`; at least 30 s). |
| `network.allow_private_targets` | `REEF_NETWORK_ALLOW_PRIVATE_TARGETS` | `false` | Let webhooks, mirrors and imports reach loopback, link-local and private addresses. |
| `limits.max_push_bytes` | `REEF_LIMITS_MAX_PUSH_BYTES` | `2147483648` | Largest `git push`, counted as git receives it (after any gzip is undone), over HTTP and SSH; 413 (HTTP) or an error message (SSH) above it. LFS has its own cap. |
| `limits.requests_per_hour` | `REEF_LIMITS_REQUESTS_PER_HOUR` | `5000` | API requests per hour for an authenticated caller (per user, shared by all their tokens and sessions). `0` = unlimited. |
| `limits.unauthenticated_requests_per_hour` | `REEF_LIMITS_UNAUTHENTICATED_REQUESTS_PER_HOUR` | `60` | API requests per hour for unauthenticated callers, per client address. `0` = unlimited. |
| `limits.web_requests_per_hour` | `REEF_LIMITS_WEB_REQUESTS_PER_HOUR` | `5000` | API requests per hour, per client address, for signed-out calls from the embedded web UI (same-origin browser requests or requests carrying the session cookie). `0` = unlimited. |

Durations are humantime strings: one or more `<number><unit>` terms with units `ms`, `s`, `m`,
`h`, `d`, `w` (`"90d"`, `"1h30m"`); `"0"` is zero.

Not configuration keys but read from the environment: `REEF_PASSWORD` (the `--password` of
`create-user`/`set-password`), `RUST_LOG`, and the variables the server sets for its own git
hooks (`REEF_DATABASE_URL`, `REEF_REPO_ID`, `REEF_PUSHER_*`, `REEF_BIN`).

## Modes

- **standalone** (default): built-in accounts with passwords; `allow_signup` controls
  self-service, and the first account created (sign-up or `reef create-user`) is the site
  administrator. An `[oidc]` section is refused.
- **oidc**: people sign in through the OpenID Connect provider named in `[oidc]`
  (authorization code flow with PKCE; discovery at the issuer). Accounts are matched by the
  provider's subject, linked by verified email when `link_by_email` is on, or created from
  the ID token (`preferred_username`, `email`, `name`). Password sign-in and sign-up are off;
  the first account created this way is the site administrator, and anyone whose ID token
  carries the boolean claim named by `admin_claim` as `true` becomes one. Reef's own API
  tokens (`reef create-token`, Settings) are how clients authenticate; the provider's
  tokens are not accepted as bearer tokens.

Register Reef at the provider once, as a confidential web client:

- redirect URI `<site_url>/login/oidc/callback`;
- post-logout redirect URI `<site_url>/`;
- back-channel logout URI `<site_url>/login/oidc/backchannel-logout` (optional; see
  [Signing out](#signing-out-single-logout));
- the client id goes in `client_id`, the secret in the file named by `client_secret_file`
  (or `REEF_OIDC_CLIENT_SECRET`).

### Changing the issuer URL

Reef records each provider identity as (issuer, subject). When the provider stays the same
but its issuer URL changes (it moved from `https://old.example.com` to
`https://id.example.com`; same provider database, same subjects), set the new `issuer` and
list the old URL:

```toml
[oidc]
issuer           = "https://id.example.com"
previous_issuers = ["https://old.example.com"]
```

On the next sign-in of each existing user, Reef finds no identity under the new issuer, finds
the subject under a listed previous issuer, moves that record to the new issuer (logged once
at `info`, "moved provider identity to the current issuer") and signs the user in to their
account. Without it, every existing user lands on the "link your account" page. A listed URL
matches with or without a trailing slash. Issuers that are not listed are never matched: list
only former URLs of this same provider, since a different provider can issue the same subject
to someone else.

To move everyone at once instead (for example before announcing the change):

```sh
reef rekey-issuer --from https://old.example.com --to https://id.example.com
```

`--to` must be exactly the `iss` the provider now puts in its ID tokens (its discovery
document's `issuer`). Identities whose subject is already linked under `--to` are left in
place and counted. Once no identity is left under the old URL, `previous_issuers` can be emptied.

### Signing out (single logout)

Ending the provider sign-in ends the Reef web session opened from it, in both directions
(OpenID Connect RP-Initiated Logout and Back-Channel Logout):

- Reef keeps the ID token's `sid` with the session it opens after a provider sign-in.
- A provider that supports back-channel logout POSTs `logout_token=<JWT>` to
  `<site_url>/login/oidc/backchannel-logout`. Reef verifies it against the provider's signing
  keys (issuer, audience `client_id`, `iat`, `exp`, the back-channel event, `sub` and/or `sid`)
  and ends the matching sessions: those of one sign-in (`sid`), or every web session of the
  account (`sub` only). It answers `200`, `400 {"error":"invalid_request"}` for a token that
  does not verify, `503` when it cannot act.
- Reef's own "Sign out" (`DELETE /api/reef/session`) ends its session and, for a session opened
  through the provider, answers `{"logout_url": "<end_session_endpoint>?id_token_hint=...&client_id=...&post_logout_redirect_uri=<site_url>/"}`
  when the provider advertises an `end_session_endpoint`.
- API tokens (`fgp_`, `fgo_`) and runner registrations are not sessions and are unaffected. A
  logout token is never accepted as a bearer token.

## Administration

```sh
reef create-user rohit rohit@example.com --password '...' --admin   # standalone mode
reef create-org acme --owner rohit
reef create-token rohit --name cli
reef set-admin rohit
reef rekey-issuer --from https://old.example.com --to https://id.example.com   # issuer URL changed
reef migrate
reef config check
```

Site administrators (the first account, `set-admin`, or the `[oidc] admin_claim`) get
`/site`: counts, users (make admin, suspend, set password), organizations, repositories,
runners and recent runs, and settings. Settings changed there (`instance_name`,
`allow_signup`, `announcement`, `default_branch`, `actions_upstream`,
`actions_retention_days`) are stored in the database and override the file until reset.

## Health, readiness, metrics, logs

- `GET /healthz` → `200 {"status":"ok"}` while the process runs.
- `GET /readyz` → `200 {"status":"ok"}` when the database answers over the dedicated health
  connection (one connection kept open apart from the application pool) and the SSH server
  is up; otherwise `503 {"status":"unavailable","failed":"database"|"ssh"|"shutdown"}` with
  no error text (the error is logged with the request id). On SIGTERM it turns 503 while the listeners
  keep serving for 5 s (so load balancers drain the instance), then the listeners stop and
  in-flight requests get 20 s.
- `GET /metrics` → Prometheus text: process metrics, `reef_build_info{version,commit}`, request
  counts and latencies by matched route, repository/user/organization counts, Actions job
  and runner gauges, webhook and mirror counters (the full list and useful queries are in
  `docs/metrics.md`). On the primary listener, or only on `[metrics].listen` when set (the
  primary then answers 404).
- These sit outside authentication, rate limiting, the 60 s request timeout and body
  limits, and log at `debug`; they are counted in the request metrics like any route.
- The 60 s request timeout exempts only routes that stream or long-poll: git smart HTTP,
  LFS, the runner protocol and source archives (routes opt out where they are defined, with
  `reef_api::timeout::exempt` or `exempt_router`; a route that says nothing is timed).
  Work that must not be cut half way (pull request merges, repository create/delete/fork,
  imports, ref, file, release and wiki writes, webhook ping/test/redelivery, the stats
  history walk) runs detached (`reef_api::timeout::detached`) in the helpers REST and
  GraphQL share: a request that times out or hangs up gets its answer cut, never the work.
  The stats history walk is single-flight: one walk per repository at a time, and concurrent
  callers wait for it.
- Every other request logs one `info` line with `msg` = `request`: `method`, `path` (no
  query), `status`, `duration_ms` (float, 3 decimals), `bytes`, `request_id`, `remote` (the
  client address, see Rate limits; an IP without port).
- Every response carries `X-Request-Id`. An inbound `X-Request-Id` is kept only from a trusted
  proxy or loopback (the Unix socket only when `[http] trusted_proxies` is set), and only when
  it matches `^[A-Za-z0-9._-]{1,64}$`; otherwise a UUID is generated.
- Logs are JSON on stderr by default (`time`, `level`, `msg`, then fields; debug and trace
  lines also carry `target`), `text` for people; `[log].file` adds a daily rolling file (14
  kept).

## Rate limits

API calls (`/api/v3`, `/api/graphql`, `/api/reef`) are rate limited the way GitHub does it:
an authenticated caller gets `[limits] requests_per_hour` (default 5000) per user, shared by
every token and session of that user; an unauthenticated caller gets
`unauthenticated_requests_per_hour` (default 60) per client address (an IPv4 address, or the
/64 of an IPv6 one; IPv4-mapped IPv6 peers count as IPv4). Each budget is a token
bucket that refills continuously (`limit / 3600` per second), so a client that stays under
the rate never sees a refusal and a burst of up to the limit is absorbed. Every limited
response carries `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Used`,
`X-RateLimit-Reset` (epoch seconds when the budget is full again) and
`X-RateLimit-Resource: core`; a refusal is `429` with the same headers, `Retry-After` and a
JSON `message`. The budget is checked before credentials are resolved, so a refused request
never costs a password hash: credentials verified in the last five minutes are charged to
their user first; others are charged to the client address first and the request is given
back once they resolve to a user (so an address that spent its budget cannot have new
credentials verified until it refills). Credentials the limiter cannot resolve (bad tokens,
or schemes it does not know such as the runner's `RemoteAuth`) are never refused by it: the
request stays charged to the address and the handler decides. An Actions job token draws
from a budget of its own job, not from the shared Actions bot user.
`GET /api/v3/rate_limit` (and GraphQL `rateLimit`) report the budget without spending it; with
the limit at `0` the endpoint answers 404, as GitHub Enterprise Server does with rate limiting
off.

Signed-out calls from the embedded web UI (a same-origin browser request, `Sec-Fetch-Site:
same-origin`, or one carrying the session cookie) are charged to a separate per-address
budget, `web_requests_per_hour` (default 5000), so browsing public repositories in the UI is
not held to the anonymous 60/h. This covers every API prefix the UI uses (`/api/v3`,
`/api/graphql`, `/api/reef`). Any client can send those headers; what that buys is the web
budget of its address, never more.

Exempt: the health endpoints, git smart HTTP, Git LFS, the runner protocol, and everything
outside `/api/` (the UI, sign-in, raw files).

The client address is the socket peer. Behind a reverse proxy list the proxy in
`[http] trusted_proxies` (addresses or CIDRs): when the peer is a trusted proxy the header
named by `[http] forwarded_header` (`x-forwarded-for`, or RFC 7239 `forwarded`) is walked
from the right past trusted hops, so a client cannot forge an entry; an unparsable hop (or
a `Forwarded` element without `for=`) stops the walk and the nearest trusted hop is the
client; when every hop is trusted the leftmost entry is the client. Without that setting every request through the
proxy shares the proxy's budget. The same address is the request log's `remote` and the
address recorded with a session. Requests on the Unix socket have no peer address: with
`trusted_proxies` set the socket counts as a trusted hop and the forwarded address is used;
otherwise unauthenticated requests on it share one budget. With `[http] proxy_protocol` a
trusted proxy's PROXY header supplies the peer address on the HTTP(S) listeners. The SSH
listener does not accept the PROXY protocol (the edge proxies only HTTPS).

Budgets are kept in memory, per instance, by design. Behind a load balancer each instance
enforces the limit on the traffic it sees, so the effective limit for a caller spread over N
instances is up to N times the configured one. At most 100 000 budgets are kept (a full map
drops its least recently used budget), and idle ones are swept on the 30 s server tick.

## Retention and cleanup

Every instance runs a retention pass at start and then hourly:

- completed workflow runs that completed more than `[actions] retention` ago (the site
  console's `actions_retention_days` overrides the file; `0` keeps everything) are removed
  with their jobs, logs and artifacts, files first, so a pass cut short is finished by the
  next one;
- artifacts past their own `expires_at` lose their file and are marked expired (they stay
  listed, as on GitHub);
- entries under `<data_dir>/tmp` untouched for 24 h are deleted;
- `reef_actions_logs_bytes` and `reef_actions_artifacts_bytes` are recomputed.

Webhook delivery logs are pruned separately after 30 days (see Webhooks). Passes are
idempotent, so several instances running them at once only share the work.

## Multi-host

Every Reef process is stateless apart from the database and the data directory. Run any
number of them behind a load balancer with the same `[database]` and a shared `data_dir`
(CephFS or NFSv4). Give each host its own `node_id` (1..15). Instance keys under
`data_dir/actions/keys/` are created once with `O_EXCL` and never regenerated: an unreadable or
truncated key file is a startup error, not a new key. Background work (webhooks, mirrors,
Actions job dispatch, retention) is coordinated through the database so every instance may
run it; section 4 of `DEPLOY.md` describes how.

## GitHub clients

Reef presents itself as a GitHub Enterprise Server host: REST at `/api/v3`, GraphQL at
`/api/graphql`, the device flow at `/login/device/code` and `/login/oauth/access_token`.

`GET /api/v3/meta` reports `installed_version` as bare semver (what `gh` and octokit parse as
the GHES version) and `version` as the full `v0.1.1 (<commit>)` string.

`gh` insists on HTTPS for every host, including `localhost`. For local development route it
through the Unix socket, which carries plain HTTP regardless of the `https://` URL:

```sh
make dev                                              # also listens on .dev/reef.sock
GH_CONFIG_DIR=.dev/ghconfig gh config set http_unix_socket $PWD/.dev/reef.sock
GH_CONFIG_DIR=.dev/ghconfig gh auth login --hostname reef.test --with-token < .dev/token
GH_CONFIG_DIR=.dev/ghconfig GH_HOST=reef.test gh repo list
```

## Tests

```sh
make test                                                     # SQLite, plus the web typecheck
REEF_TEST_DATABASE_URL=postgres://... cargo test -p reef-db   # data layer on PostgreSQL as well
```

## Wikis, releases, projects

- Repository wikis live in `<repo>.wiki.git`; organization wikis in `<org>.wiki.git` (members edit, everyone reads). Both are served at `/api/reef/{repos/{o}/{r}|orgs/{org}}/wiki`.
- Releases follow the GitHub REST shape (`/repos/{o}/{r}/releases`, `/releases/latest`, `/releases/tags/{tag}`); publishing creates the tag at the target when it does not exist. Source archives: `/repos/{o}/{r}/{tarball|zipball}/{ref}`.
- Files can be created, edited and deleted in the browser or through `PUT`/`DELETE /repos/{o}/{r}/contents/{path}`; branch rules that require a pull request or lock the branch apply.
- Projects: views carry `settings` (`column_field_id`, `group_by_field_id` for swimlanes/groups, `sort`, `visible_fields`, `filter`). Built-in workflows set Status to the first option when an item is added, Done when its issue closes, and Todo when it reopens.

## Actions

Runner protocol notes are in `docs/02-runner-protocol.md`. Tenant URLs are `<site_url>/_actions/{repo|org}/{id}`; job logs and artifacts live under `<data_dir>/actions` (shared storage for multi-host). Secrets are sealed with libsodium boxes on upload (`gh secret set`) and AES-GCM at rest under `<data_dir>/actions/keys`. The runner protocol routes (long polls, uploads) are outside the request timeout.

Known gaps: artifact upload (`actions/upload-artifact`), the cache service, reusable workflows, scheduled (`cron`) triggers, and job/step annotations in the UI.

## Notifications

Notifications are created for mentions, assignments, review requests, comments and state changes on issues and pull requests you take part in, submitted reviews, and failed workflow runs you triggered. They are at `/notifications`, the bell in the header, and GitHub's `/notifications` REST shape.

## Insights and contributions

Repository insights are computed from one cached index of the default branch's history (`git log --numstat`, up to 20 000 non-merge commits, refreshed when the branch tip moves or after ten minutes). Commit authors map to accounts by e-mail, then by author name equal to a login; everyone else is reported as `{"login": null, "name", "email", "type": "Anonymous"}`.

- GitHub shapes under `/api/v3/repos/{o}/{r}/stats/`: `contributors`, `commit_activity`, `code_frequency`, `participation` and `punch_card`. Weeks start on Sunday 00:00 UTC.
- `GET /api/reef/repos/{o}/{r}/pulse?days=1|3|7|30`, `GET /api/reef/repos/{o}/{r}/tree-commits?rev=&path=`, `GET /api/reef/users/{login}/contributions?from=&to=` (cached for two minutes; counts only repositories the viewer can read).

The primary language of a repository is stored in the database and refreshed after pushes to the default branch, after the initial commit, and whenever `/repos/{o}/{r}/languages` runs.

## Webhooks

Repository and organization webhooks follow GitHub's API (`/repos/{o}/{r}/hooks`, `/orgs/{org}/hooks`, deliveries, `pings`, `tests`, redelivery) and UI (`Settings → Webhooks`). Deliveries carry `X-GitHub-Event`, `X-GitHub-Delivery`, `X-Hub-Signature-256` and `User-Agent: GitHub-Hookshot/reef`. Events are taken from the activity tables by a poller on the 30 s server tick; failed deliveries retry after 1, 5 and 30 minutes (network errors and 5xx only) and the delivery log keeps 30 days. Recording an event also queues it in the webhook outbox in the same transaction; the poller claims outbox entries under a lease, and records an entry's deliveries and deletes the entry in one transaction, so every event becomes deliveries exactly once, a process dying half way loses nothing, and an event that commits late (after events with larger ids, as happens across instances) is still delivered. While no hook exists the outbox is emptied, so enabling a hook never replays history. Deliveries are posted under a lease (`FOR UPDATE SKIP LOCKED` on PostgreSQL) that the dispatcher renews before each post.

## Identity: emails, GPG and SSH signing keys

Accounts can have several emails (`Settings → Emails`, GitHub's `/user/emails` API). Commits are attributed to the account that owns the author email, or whose login equals the author name. GPG keys and SSH signing keys (`/user/gpg_keys`, `/user/ssh_signing_keys`) verify commit signatures; signed commits show a "Verified" or "Unverified" badge and the commit JSON carries GitHub's `verification` object. SSH signatures need `ssh-keygen` on the server; the scratch files go under `<data_dir>/tmp`.

## Git LFS, mirrors and import

LFS works with the stock `git lfs` client over HTTPS and SSH (`git-lfs-authenticate`), including file locks. `Settings → Mirrors and LFS` configures a pull mirror (the repository becomes read-only and syncs on an interval), push mirrors (pushed after every push), and shows LFS usage and locks. `/new/import` and GitHub's `PUT /repos/{o}/{r}/import` clone a repository from another Git host into an empty repository. Mirror URLs are never logged (they may carry credentials).

## Renaming and deleting accounts

Site administrators rename users and organizations from `/site`; users rename themselves under `Settings → Public profile`; organization owners rename or delete an organization under `Settings → Rename or delete` (`DELETE /orgs/{org}` needs the `admin:org` scope). Renames are immediate and are not redirected from the old name.

## Reserved logins

Logins that would collide with routes or the product are reserved (`site`, `root`, `api`, `login`, `settings`, `reef`, `healthz`, `readyz`, `metrics`, and the rest of `reef-core::names::RESERVED_LOGINS`).
