<p align="center">
  <img src="web/public/logo.svg" width="96" alt="Reef">
</p>

<h1 align="center">Reef</h1>

<p align="center">
  A GitHub-compatible developer platform in a single binary.<br>
  Git hosting, issues and pull requests, Actions-compatible CI, projects, wikis and organizations, for your own servers.
</p>

<p align="center">
  <a href="https://github.com/yadvlabs/reef/actions/workflows/ci.yml"><img src="https://github.com/yadvlabs/reef/actions/workflows/ci.yml/badge.svg" alt="ci"></a>
  <a href="https://hub.docker.com/r/yadvlabs/reef"><img src="https://img.shields.io/docker/v/yadvlabs/reef?label=docker&sort=semver" alt="docker"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT"></a>
</p>

> **Experimental.** Reef is early software, at version 0.1. It works for day-to-day use, but
> APIs, configuration keys and the database schema may still change between releases without
> an upgrade path, and there are known gaps (see [Status](#status)). Try it, break it, and
> file issues. Keep a copy of anything you cannot afford to lose somewhere else for now.

## What it does

Reef looks like GitHub and speaks GitHub's REST and GraphQL API, so `gh`, octokit, IDE
integrations and the stock `actions/runner` work against it unchanged.

- **Repositories**: git over HTTPS and SSH, branch protection, releases with source archives,
  Git LFS, pull and push mirrors, imports from other hosts, webhooks with GitHub's payloads.
- **Collaboration**: issues, labels, milestones, pull requests with reviews and merges,
  notifications, wikis, project boards with tables, swimlanes and workflows.
- **Actions**: workflows in `.github/workflows` run on stock `actions/runner` binaries you
  register with a token; actions from github.com are fetched through Reef.
- **Organizations** with members and roles, personal and org-level settings, a site
  administration console, insights and contribution graphs.
- **Operations**: one static binary with the web UI embedded, SQLite out of the box,
  PostgreSQL for several instances behind a load balancer, structured logs, Prometheus
  metrics, readiness probes, TLS termination, rate limiting.
- **Sign-in**: built-in accounts with passwords, or any OpenID Connect provider with single
  logout.

Written in Rust (axum, sqlx, gix, russh) with a React and Primer UI.

## Getting started

### Docker

```sh
docker run -d --name reef \
  -p 4920:4920 -p 4922:4922 \
  -v reef-data:/var/lib/reef \
  yadvlabs/reef
```

Open http://localhost:4920 and create the first account. It becomes the site administrator.
Images are published for linux/amd64 and linux/arm64. Every setting is an environment
variable, so for a real host add `-e REEF_SITE_URL=https://reef.example.com` and put a
reverse proxy with TLS in front. [`deploy/docker-compose.yml`](deploy/docker-compose.yml) is
the same with an optional PostgreSQL service.

### Binary

Each [release](https://github.com/yadvlabs/reef/releases) ships `reef-<version>-linux-<arch>.tar.gz`
with a static binary, a systemd unit and an annotated configuration file. It needs only
`git` on the host (plus `git-lfs` for LFS and `ssh-keygen` for SSH-signed commits).

```sh
tar xzf reef-v0.1.0-linux-amd64.tar.gz && cd reef-v0.1.0-linux-amd64
REEF_DATA_DIR=$PWD/reef-data bin/reef serve         # try it on http://localhost:4920
```

To install it as a service:

```sh
sudo install -m 0755 bin/reef /usr/local/bin/reef
sudo useradd --system --home-dir /var/lib/reef --shell /usr/sbin/nologin reef
sudo mkdir -p /etc/reef && sudo cp conf/reef.conf.example /etc/reef/reef.conf   # set site_url at least
sudo cp units/reef.service /etc/systemd/system/ && sudo systemctl enable --now reef
```

### From source

```sh
git clone https://github.com/yadvlabs/reef && cd reef
make build-native        # needs Rust 1.95, Node 24 with pnpm, cmake; result in bin/host/reef
```

[DEVELOPMENT.md](DEVELOPMENT.md) describes the toolchain and the dev loop.

### First steps

1. **Create the first account** in the browser (or `reef create-user --admin`). Self-service
   sign-up is off by default; turn it on with `allow_signup = true` or from the site console.
2. **Use the GitHub CLI**: mint a token under *Settings → Tokens* (or `reef create-token`),
   then `gh auth login --hostname reef.example.com --with-token`.
3. **Clone over SSH**: add your public key under *Settings → SSH keys* and use
   `ssh://git@reef.example.com:4922/owner/repo.git`.
4. **Run CI**: in a repository go to *Settings → Actions*, download the stock runner, and
   register it with the token shown there. Runners talk to Reef on port 80 or 443, so expose
   it through the proxy. See [DEPLOY.md](DEPLOY.md), section 6.

## Ports and URLs

| What | Where |
| --- | --- |
| HTTP(S): web UI, API, git over HTTP, runners | `4920` |
| SSH | `4922`: `ssh://git@host:4922/org/repo.git` |
| Metrics (optional, loopback) | `4924` via `[metrics].listen` |
| Health | `GET /healthz`, `GET /readyz`, `GET /metrics` |
| REST / GraphQL | `/api/v3`, `/api/graphql` (or an `api.` host through `api_url`) |

## Configuration

`reef serve --config /etc/reef/reef.conf` reads a TOML file (optional), `conf.d/*.conf`
drop-ins, and a `REEF_<SECTION>_<KEY>` environment variable for every key. Secrets come from
`*_file` keys or their value twins (`REEF_DATABASE_PASSWORD`, `REEF_OIDC_CLIENT_SECRET`).
`reef config check` prints the effective configuration with secrets redacted.

Two sign-in modes, chosen by `[auth] mode`: `standalone` (default, built-in accounts) and
`oidc` (any OpenID Connect provider named in `[oidc]`, with an optional claim that grants
site administrator).

Every key with its default and environment twin: [docs/configuration.md](docs/configuration.md).
A complete annotated file: [deploy/reef.conf.example](deploy/reef.conf.example).

## Documentation

- [DEPLOY.md](DEPLOY.md): TLS and reverse proxies, PostgreSQL, several instances, upgrades, containers, Actions runners.
- [docs/configuration.md](docs/configuration.md): every setting, administration, health and metrics, rate limits, retention.
- [DEVELOPMENT.md](DEVELOPMENT.md): building, the dev server, tests, conventions, releasing.
- [docs/metrics.md](docs/metrics.md), [docs/02-runner-protocol.md](docs/02-runner-protocol.md), [docs/00-research.md](docs/00-research.md).

## Status

Reef is used daily by its authors and is tested on SQLite and PostgreSQL in CI, including an
end-to-end run over REST, GraphQL, git over HTTP and SSH. Not there yet: Actions artifact
upload and the cache service, reusable workflows and scheduled triggers, package registries,
discussions, code search, and rename redirects. [TODO.md](TODO.md) has the full backlog.
Expect rough edges and report them.

## Contributing and security

Pull requests are welcome; [CONTRIBUTING.md](CONTRIBUTING.md) has the checklist and
[DEVELOPMENT.md](DEVELOPMENT.md) the setup. Report vulnerabilities privately as described in
[SECURITY.md](SECURITY.md).

## License

MIT. Copyright (c) 2026 Rohit Yadav and YADV Labs authors. Reef and Crusty the crab are the
project's own branding.
