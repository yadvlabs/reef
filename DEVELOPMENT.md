# Developing Reef

Everything a contributor does goes through `make`: it is the one entry point that knows how
the web UI, the Rust workspace, the local dev server, the tests and the release artifacts fit
together, so nobody has to remember the order of `pnpm build`, `cargo build`, config files and
pidfiles. `make help` lists every target with a one-line description.

## Prerequisites

| Tool | Version | Used for |
| --- | --- | --- |
| Rust toolchain | 1.95 (`rust-toolchain` via rustup) | the server and CLI |
| Node.js with pnpm | Node 24, pnpm 12 (`corepack enable` or `npm i -g pnpm`) | the web UI under `web/` |
| `git`, `ssh-keygen` | any recent | the server shells out to git; SSH signing keys need ssh-keygen |
| `cargo-zigbuild` and zig | latest | static Linux binaries for both architectures (`make build`, `make release`) |
| `cmake` | any recent | builds the `aws-lc-sys` crypto dependency |
| Docker with buildx | optional | `make image`, `make image-push` |
| PostgreSQL | optional | run the data layer tests against Postgres as well |

On macOS: `brew install cmake zig pnpm` and `cargo install cargo-zigbuild`. On Debian or
Ubuntu: `apt install cmake` and `pipx install cargo-zigbuild` (the pip package bundles zig).

## Layout

```
bin/reef             the binary: serve, migrate, config check, version, print-unit, account commands, git hook entry point
crates/reef-core     ids, time, errors, tokens, naming rules
crates/reef-db       sqlx data layer (SQLite and PostgreSQL from one SQL dialect), migrations/
crates/reef-git      repository store, git smart protocol, reading with gix, server-side hooks
crates/reef-ssh      git over SSH (russh)
crates/reef-api      axum HTTP: REST /api/v3, GraphQL /api/graphql, git HTTP, sessions, OIDC, health, the embedded UI, configuration
crates/reef-actions  workflow model: YAML, triggers, matrices, expressions
web/                 React 19 + Primer React + TanStack Router/Query, built by Vite, embedded into the binary from web/dist
deploy/              systemd unit, Dockerfile, docker-compose.yml, reef.conf.example
scripts/             dev-runner.sh (stock actions/runner against the dev server), e2e.sh
docs/                architecture research, configuration reference, runner protocol, metrics
```

The web UI is embedded at compile time (`rust-embed` over `web/dist`), so a server build
picks up whatever `make web` last produced. A fresh clone compiles without the UI; the server
then shows a placeholder page until the UI is built.

## The dev loop

```sh
make dev            # build UI + debug binary, start on http://localhost:4920 with built-in accounts, follow the log; Ctrl+C stops it
make web-dev        # in a second terminal: Vite with hot reload on :5173, proxying API and git traffic to :4920
```

The first account you create is the site administrator. Dev state lives under `.dev/`:
`reef.conf` (regenerated on every start; put overrides in `.dev/conf.d/*.conf`), `data/`
(the SQLite file, repositories, hooks, keys), `reef.log`, `reef.pid`, `reef.sock`. `make dev`
turns on `allow_signup`, text logs, `file://` mirror sources and private-network targets for
webhooks, none of which are defaults in production.

Background control: `make dev-start`, `dev-stop`, `dev-status`, `dev-logs`, `dev-restart`.
`make reset` deletes the dev database, repositories and generated config.

To work on OpenID Connect sign-in, run `make dev-oidc OIDC_ISSUER=https://id.example.com`
with the client secret in `.dev/oidc-client-secret` (or `REEF_OIDC_CLIENT_SECRET` in the
environment) after registering `http://localhost:4920/login/oidc/callback` at the provider.

### Using `gh` against the dev server

`gh` insists on HTTPS, so route it through the Unix socket, which carries plain HTTP:

```sh
target/debug/reef --config .dev/reef.conf create-token <login> --name dev > .dev/token
GH_CONFIG_DIR=.dev/ghconfig gh config set http_unix_socket $PWD/.dev/reef.sock
GH_CONFIG_DIR=.dev/ghconfig gh auth login --hostname reef.test --with-token < .dev/token
GH_CONFIG_DIR=.dev/ghconfig GH_HOST=reef.test gh repo list
```

### Actions runners

Stock `actions/runner` binaries drop the port from `--url`, so the dev server must also
listen on :80 for them: `make dev-runners` (dev-start plus `:80`), then
`make runner REPO=owner/name` downloads, registers and runs a runner in the foreground using
the token in `.dev/token`.

## Tests and checks

```sh
make lint           # cargo fmt --check, clippy -D warnings
make test           # cargo test on SQLite, then the web typecheck
make e2e            # builds, starts a throwaway server, drives it over REST, GraphQL, git over HTTP and SSH
make check          # lint + test, what CI runs before building
REEF_TEST_DATABASE_URL=postgres://reef:pw@localhost/reef_test make test-rust   # data layer on PostgreSQL too
```

A throwaway PostgreSQL for that last line, with no system service involved (Homebrew's
`postgresql@17` or any distribution package):

```sh
PG=/opt/homebrew/opt/postgresql@17/bin                       # or wherever initdb and pg_ctl live
$PG/initdb -D /tmp/reef-pg -U reef --auth=trust -q
$PG/pg_ctl -D /tmp/reef-pg -o "-p 54329 -k /tmp -c listen_addresses=127.0.0.1" -l /tmp/reef-pg/log start -w
$PG/createdb -h 127.0.0.1 -p 54329 -U reef reef_test
REEF_TEST_DATABASE_URL="postgres://reef@127.0.0.1:54329/reef_test?sslmode=disable" cargo test -p reef-db
$PG/pg_ctl -D /tmp/reef-pg stop -w && rm -rf /tmp/reef-pg
```

The data-layer tests share that one database, so the ones that empty it run one at a time.

`scripts/e2e.sh` is the smoke test for the basic feature set: sign-up, tokens, repositories,
push and clone over HTTP and SSH, issues, pull requests and merges, releases, wikis,
organizations, GraphQL, metrics. It needs only the debug binary, `git`, `ssh` and `python3`,
and runs in CI after the unit tests.

## Conventions

- **GitHub compatibility first.** REST shapes, paths and error messages mirror GitHub's so
  `gh`, octokit and the stock runner keep working. New endpoints go under `/api/v3` with
  GitHub's shape; Reef-only features live under `/api/reef`.
- **Configuration keys** are struct fields in `crates/reef-api/src/config.rs`; every key gets
  an environment twin automatically. Add the key, its default, a line in
  `deploy/reef.conf.example` and a row in `docs/configuration.md`. Secrets are `*_file` keys
  with a value twin, never inline.
- **Database changes** are new numbered files under `crates/reef-db/migrations/`, written
  in the portable SQL dialect both backends accept (BIGINT ids and epoch-millisecond
  timestamps, TEXT for JSON). Expand and contract: a release may add, and may only drop what
  the previous release no longer reads, so binary N runs against schema N+1. Bump
  `SCHEMA_RANGE` in `crates/reef-db/src/lib.rs` with the migration. Never edit an applied
  migration; sqlx checksums them.
- **Logging** is structured JSON on stderr (`time`, `level`, `msg`, fields); `[log] format =
  "text"` for people. One `request` line per request with the request id.
- **Formatting** is `cargo fmt` with the repository's `rustfmt.toml`; clippy must be clean
  with warnings denied.

## Building and releasing

```sh
make build          # static release binaries for linux/arm64 and linux/amd64 into bin/linux-<arch>/ (web UI embedded)
make build-native   # release binary for this machine into bin/host/
make release VERSION=v0.1.0   # the two tarballs plus SHA256SUMS under dist/
make image          # container image for this machine's architecture
make image-push IMAGE=yadvlabs/reef:0.1.0   # multi-arch image built with buildx and pushed
```

The version lives in three places that must move together: `Cargo.toml`
(`workspace.package.version`), `web/package.json` and the Makefile's `VERSION` fallback.
`make release` refuses a `VERSION` that is not the Cargo version or a pre-release of it. The
binary stamps `v<version> (<commit>)`, with `-dirty` on the commit when tracked files have
changes; `reef version`, `/api/v3/meta` and the `reef_build_info` metric report it.

Releasing: bump the three version fields, commit, tag `vX.Y.Z` and push the tag. The
`release` workflow builds the tarballs, attaches them to a GitHub release, and pushes the
multi-arch image to Docker Hub (it needs the `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`
secrets). Every push also runs `ci.yml`: lint, tests on SQLite and PostgreSQL, the e2e
script, the cross builds and an image build without push.

## Troubleshooting

- *"migration 1 was applied by a different build"*: the dev database was created from
  different migration files. `make reset` and start again.
- *The UI shows "Reef is running" but nothing else*: the binary was built before `make web`.
  Run `make web` and rebuild.
- *Runner registration fails with a connection error*: the runner is talking to port 80.
  Use `make dev-runners`, or put a proxy on 80/443 in front of a real deployment.
- *`gh` refuses the dev server*: it wants HTTPS; use the Unix socket recipe above.
