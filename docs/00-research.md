# Reef: research and proposed architecture

Draft, 27 September 2026. Written before any code. Versions were checked against crates.io, npm, and upstream repositories on that date.

Reef is a self-hosted developer platform: code hosting (git), collaboration (issues, pull requests), automation (CI and CD), project management (kanban boards, task tracking, wikis), governance (organizations, teams), and package and registry hosting (build artifacts for lab, product, and releases). It looks and feels like GitHub, and it speaks GitHub's API so that GitHub clients work against it unchanged.

Language is Rust. The database is SQLite or PostgreSQL, chosen at install time.

## 1. The one decision that shapes everything: be a GitHub Enterprise Server host

Every GitHub client already has a mode for "GitHub Enterprise Server" (GHES) hosts. That mode is the cheapest compatibility target, and it is what Reef presents:

| Surface | Path on Reef | Who uses it |
| --- | --- | --- |
| REST | `https://HOST/api/v3/` | gh, octokit, go-github, Terraform provider, Renovate, Dependabot, actions/checkout, actions/github-script |
| GraphQL | `https://HOST/api/graphql` | gh (most porcelain), VS Code Pull Requests extension |
| OAuth device flow | `/login/device/code`, `/login/oauth/access_token` | `gh auth login --hostname HOST` |
| Token check | `GET /api/v3/user` with `X-OAuth-Scopes` response header | gh, git-credential-manager, git-credential-oauth |
| Git smart HTTP | `/{owner}/{repo}.git/info/refs`, `git-upload-pack`, `git-receive-pack` | git |
| Git SSH | `git@HOST:owner/repo.git` | git |
| Webhooks | outbound, `X-GitHub-Event`, `X-GitHub-Delivery`, `X-Hub-Signature-256` | anything that consumes GitHub webhooks |

Facts that follow from this:

- **Do not model on Gitea's `/api/v1`.** It is GitHub-shaped but not GitHub-compatible (different field names, pagination, label colours). No GitHub-native client works against it. That is why `gh` does not work on Gitea or Forgejo, and nobody has fixed it server-side.
- **GraphQL is mandatory for `gh`.** `gh pr create/view/list/edit/checks`, `gh issue list/view`, `gh repo view/create/clone`, and `gh auth login` are GraphQL. Only `gh run`, `gh release`, PR merge, fork, and rename are REST. `gh` also runs `__type` introspection queries against enterprise hosts for feature detection, so the GraphQL server must be schema-backed (async-graphql gives introspection for free).
- **The spec is available and MIT.** `github/rest-api-description` publishes OpenAPI 3.0 and 3.1 documents per product (`api.github.com`, `ghec`, `ghes-3.x`). The 3.1 document also carries every webhook payload schema. The `api.github.com` bundle is about 13 MB and roughly 1000 operations. Generate serde types from it for the schemas we serve; hand-write the axum handlers. Do not attempt to generate a server for the whole spec.
- **Versioning.** Honour `X-GitHub-Api-Version`. Support `2022-11-28` (default) and `2026-03-10` (first breaking calendar version, March 2026). Serve `GET /versions`. Return 410 for anything else.
- **Tokens.** `gh` uses `GH_ENTERPRISE_TOKEN` for non-github.com hosts, and `GH_HOST` when there is no git remote. Pasted tokens must report `repo` and `read:org` in `X-OAuth-Scopes`. Personal access tokens carry a recognisable prefix and are stored as a SHA-256 hash for O(1) lookup.

### Minimal GraphQL surface for `gh`

Queries: `viewer`, `repository(owner, name)`, `repositoryOwner`, `search` (issues and pull requests), `node(id)`, `rateLimit`, `__type`, `__schema`.

Mutations: `createRepository`, `updateRepository`, `cloneTemplateRepository`, `createPullRequest`, `updatePullRequest`, `closePullRequest`, `reopenPullRequest`, `markPullRequestReadyForReview`, `convertPullRequestToDraft`, `requestReviews`, `replaceActorsForAssignable`, `updateProjectV2Items`, plus the issue equivalents.

IDs: Relay-style opaque `id` on every node plus the integer `databaseId`. Both must be stable.

## 2. Git: gix inside the repository, the git binary on the wire

This is what every production reef does (Gitea, Forgejo, GitLab's Gitaly, Radicle, Josh). No Rust library ships a battle-tested server side of the smart protocol.

| Concern | Choice | Notes |
| --- | --- | --- |
| Read objects, trees, diffs, blame, merge-base, rev-walk, ref updates, object writes | `gix` 0.88 (gitoxide) | Sync API. `Repository` is Send, not Sync: keep a `ThreadSafeRepository` in server state and call `to_thread_local()` inside `spawn_blocking`. Enable `max-performance`. |
| Rename-aware blame, textual patch output | `git2` 0.21 (libgit2 1.9.6) as an optional fallback | Disable its `ssh` and `https` features; we never need libgit2 networking. |
| Smart HTTP | spawn `git upload-pack --stateless-rpc` and `git receive-pack --stateless-rpc` from axum handlers | Forward `GIT_PROTOCOL` for protocol v2, gunzip `Content-Encoding: gzip` bodies, stream request body to child stdin and child stdout to the response, kill the child on client disconnect. Content types `application/x-git-{svc}-{advertisement,request,result}`. About 150 lines. |
| SSH | `russh` 0.63 server | Handle `exec_request`, accept only `git-upload-pack '<path>'` and `git-receive-pack '<path>'`, spawn git, pipe the channel to stdio. Needs exactly one crypto backend feature (`aws-lc-rs` or `ring`). Used in production by Warpgate, GitArena, rgit. |
| Hooks | one central `core.hooksPath` whose scripts exec the `reef` binary | The hook binary reads ref updates from stdin and identity from environment variables set by the HTTP or SSH handler, then calls the server's internal API to check branch protection (`pre-receive`) and to fire events (`post-receive`). This is Gitea's design; Forgejo is moving to the central hooks path, so start there. |
| LFS | own batch endpoint (`POST .../info/lfs/objects/batch`) with local or S3 object storage | The spec is small. The `git-lfs-api` crate has typed request and response structs. Locks API later. |

Known gaps in gix as of September 2026: no push, no receive-pack or upload-pack, no reftable, no delta compression when writing packs, and blame does not follow renames. None of these matter for a server that uses the git binary for transport.

Runtime dependency: the `git` binary must be installed on the Reef host. This is the same requirement Gitea has.

Reference codebases: GitArena (actix, sqlx, gix, russh; the best HTTP plus SSH reference in Rust), rgit by w4 (axum, gix, read-only browsing of large repositories), Josh (axum, gix, git proxying used by rust-lang), and Gitea's `routers/web/repo/githttp.go`.

## 3. Backend stack

| Concern | Choice | Version | Why |
| --- | --- | --- | --- |
| HTTP | axum | 0.8.9 | tower ecosystem, WebSocket and SSE built in, streaming bodies both ways for git packs and multi-gigabyte package uploads, per-route body limits. |
| Database | sqlx `Any` driver, hand-written portable SQL, app-generated 64-bit IDs | 0.9 | One codebase, both SQLite and PostgreSQL, backend chosen from the URL at runtime. IDs are generated in the application (time-ordered, node-tagged) so there is no autoincrement dialect gap and no coordination between hosts. Timestamps are integer epoch milliseconds, booleans are integers, JSON is text. `$1` placeholders work on both engines. A thin `Db` wrapper exposes the backend kind for the few divergent statements. |
| Why not sea-orm | | 2.0 | Considered first. Rejected for the first slices: the entity and migration DSL is large for a schema of this size, and the ID scheme above already removes the main portability problem. It remains the fallback if hand-written SQL becomes a burden. |
| Why not diesel | | 2.3 | Multi-backend needs generic bounds on every query function; SQLite is async only through a sync wrapper. |
| Search | tantivy | 0.26 | Embedded, one implementation for both databases. Issues, pull requests, wiki, and a per-file code index with a code tokenizer. Trigram regex search later. |
| Jobs | in-process tokio workers over a `jobs` table | | `FOR UPDATE SKIP LOCKED` on PostgreSQL, `BEGIN IMMEDIATE` on SQLite, lease column for multi-replica. tokio-cron-scheduler for schedules. Adopt apalis once 1.0 leaves release candidate if we need distributed workers. Mirrors Gitea's database-backed queue. |
| Auth | openidconnect 4, webauthn-rs 0.5, totp-rs 6, argon2 0.6, tower-sessions 0.15, ssh-key 0.6, rPGP 0.20 | | OIDC and OAuth2 login, passkeys, TOTP, password hashing, sessions in the database, SSH key parsing, SSH and PGP commit signature verification. |
| Markdown | comrak 0.55 with ammonia | | GFM (tables, task lists, footnotes, strikethrough, autolinks, math). Safe by default. Used by GitLab and crates.io. |
| Highlighting | syntect 5.3 with two-face 0.5 | | Pure Rust, parallel-safe. tree-sitter later for symbol extraction only. |
| Diff | imara-diff 0.2 for repository diffs, similar 3.2 for word-level highlighting | | Histogram and Myers, git-like output. |
| Registry types | oci-spec, cargo registry API structs | | See section 5. |
| Config | config 0.15 | | Defaults, then TOML, then environment, then CLI flags (clap). |
| Telemetry | tracing, opentelemetry 0.33, tracing-opentelemetry 0.34, Prometheus metrics | | |
| Embedding the UI | rust-embed 8 with axum-embed | | Reads from disk in debug builds, embeds in release builds, serves precompressed assets. One static binary. |
| Release builds | musl targets via cargo-zigbuild, rustls everywhere, bundled SQLite, fat LTO, stripped | | No OpenSSL. |

Server-rendered content: markdown, diffs, and syntax highlighting are produced in Rust and shipped to the browser as sanitised HTML. GitHub does the same. It keeps trust decisions on the server and removes the need for a client-side highlighter.

## 4. Frontend: Primer React

The requirement is that Reef looks and feels like GitHub. GitHub's own design system is open source under MIT and is the direct route:

| Package | Version | Notes |
| --- | --- | --- |
| @primer/react | 38.40 | Published 18 September 2026. React 18 or 19, ESM only, CSS Modules. `sx` and styled-components were removed in v38. |
| @primer/primitives | 11.10 | Tokens and themes, including dark mode. |
| @primer/octicons-react | 19.38 | GitHub's icons. |
| @primer/css | 22.3 | Maintenance mode. Do not build on it. |

Primer React ships PageLayout, DataTable, ActionMenu, Timeline, UnderlineNav, Blankslate, and the rest of the GitHub vocabulary. Interaction-heavy screens (kanban board, PR review, live CI logs, diff viewer) justify a single-page application: React 19.3, Vite 8.3, TanStack Router 1.170, TanStack Query 5.104. This matches the POC console's tooling, so the two front ends share build practice even though they do not share components.

Branding: the brief is GitHub fidelity, so Reef keeps Primer's layout vocabulary and limits its own identity to the logo and a small Primer theme overriding accent colour and font tokens. Decide this before the first screen is built.

Server-side rendering with askama or maud plus htmx was considered and rejected for the core app. It wins only if the team is Rust-only and public-repository SEO matters; it doubles the templating work for a product whose hardest screens are interactive.

## 5. CI and CD: GitHub Actions compatible

Workflow YAML is the contract. Existing actions (`actions/checkout`, `actions/setup-node`, `actions/upload-artifact@v4`, `actions/cache`) must work.

What the server has to provide for actions to run at all:

| Variable | Server-side service |
| --- | --- |
| `GITHUB_API_URL`, `GITHUB_SERVER_URL`, `GITHUB_TOKEN` | the `/api/v3` REST surface; checkout, github-script, and release actions call it |
| `ACTIONS_RUNTIME_URL`, `ACTIONS_RUNTIME_TOKEN` | JWT-authenticated runtime endpoint |
| `ACTIONS_RESULTS_URL` | twirp `github.actions.results.api.v1.ArtifactService` (artifact v4) and `CacheService` v2 with Azure-style block upload |
| `ACTIONS_CACHE_URL`, `ACTIONS_CACHE_SERVICE_V2=true` | cache v1 REST for older actions, v2 twirp for current ones |
| `ACTIONS_ID_TOKEN_REQUEST_URL`, `ACTIONS_ID_TOKEN_REQUEST_TOKEN` | OIDC issuer: `.well-known/openid-configuration`, JWKS, RS256 tokens with `sub`, `repository`, `ref`, `workflow`, `run_id`, `environment`, `job_workflow_ref` |

Runner strategy, in order:

1. **Server side in Rust from day one**: workflow parsing and expression evaluation (zizmor's `github-actions-models` and `github-actions-expressions` crates), scheduling, matrix expansion, reusable workflows, secrets, the results, artifact, cache, and OIDC services above, and the REST endpoints `gh run` uses (`actions/runs`, `attempts/{n}/jobs`, `actions/jobs/{id}`, `check-runs/{id}/annotations`).
2. **Runner protocol, phase one: Gitea's Connect-RPC `runner.v1.RunnerService`** (Register, Declare, FetchTask, UpdateTask, UpdateLog). It is documented in `gitea/actions-proto-def`, and two mature runners already speak it: Forgejo runner (a fork of nektos/act) and ChristopherHX's bridge to GitHub's real `actions/runner`. This gets workflows executing in weeks, not months.
3. **Runner protocol, phase two: GitHub's own `actions/runner` protocol** (registration, long-poll message broker, RSA-encrypted job messages, twirp results). `ChristopherHX/runner.server` is a complete reimplementation of the server side and the reference to port. This lets customers use stock GitHub runners and runner images.
4. **A Rust runner** later, reusing the crates from step 1. `Falconiere/toolu-ghrunner` shows the shape.

Known gaps in Gitea and Forgejo Actions to avoid repeating: expression functions missing in some contexts, `runs-on` limits, no problem matchers, and no `statuses`, `checks`, `id-token`, or `pages` permission scopes. Some actions check that the host is GitHub; Gitea's runner patches that check out of the action bundle.

## 6. Packages and registries

Order of implementation, driven by what a typical team's build needs (container images, Rust crates, Go modules, Debian packages for host installs, generic release artifacts):

| Format | Protocol | Rust building blocks |
| --- | --- | --- |
| Container and Helm | OCI Distribution v1.1 | `oci-spec` for types; `trow` (axum, passes the conformance suite) as a reference |
| Generic | plain PUT and GET with checksums | hand-written |
| Cargo | sparse index (`config.json` with `dl`, `api`, `auth-required`), `PUT /api/v1/crates/new`, 401 with `X-Cargo-Token-Url` | kellnr, cratery, and `registry_api` as references |
| Debian and RPM | apt and dnf repository layouts with signed metadata | hand-written |
| Go | `GOPROXY` (`/@v/list`, `.info`, `.mod`, `.zip`) | hand-written |
| npm | packument plus tarball, `PUT /{pkg}` | hand-written |
| PyPI | PEP 503 and 691 simple index plus legacy upload | hand-written |
| Maven | PUT and GET plus `maven-metadata.xml` | hand-written |
| NuGet | v3 service index | hand-written |

Gitea and Forgejo support 23 formats under one path scheme; GitHub Packages supports five. Package clients speak standard protocols, so this is the one area where compatibility is inherent. Blob storage is a single content-addressed store (local filesystem or S3-compatible) shared by LFS, packages, CI artifacts, and the cache.

## 7. Webhooks

Copy GitHub exactly: `X-GitHub-Event`, `X-GitHub-Delivery` (GUID), `X-GitHub-Hook-ID`, `X-GitHub-Hook-Installation-Target-Type` and `-ID`, `User-Agent: GitHub-Hookshot/reef`, `X-Hub-Signature: sha1=` and `X-Hub-Signature-256: sha256=` HMAC over the raw body. Ten-second delivery timeout, 25 MB payload cap, deliveries stored and redeliverable. Payload shapes come from the OpenAPI 3.1 `webhooks` object. Expose `repos/{owner}/{repo}/hooks` and `/deliveries` over REST.

## 8. Proposed workspace layout

One Cargo workspace, one binary, crates per domain:

```
reef/
  Cargo.toml                 workspace, [workspace.dependencies], cargo-deny
  crates/
    reef-core/              ids, errors, events, global ID encoding
    reef-db/                sqlx Any pool, migrations, Db abstraction, ID generator
    reef-storage/           content-addressed blob store (fs, s3)
    reef-git/               gix wrappers, smart HTTP and SSH transport, hooks
    reef-auth/              sessions, PATs, OAuth device flow, OIDC, passkeys
    reef-tenancy/           users, organizations, teams, permissions
    reef-repos/             repository model, branches, protection, LFS
    reef-issues/            issues, labels, milestones, comments, reactions
    reef-pulls/             pull requests, reviews, merge strategies, checks
    reef-projects/          kanban boards, views, fields (ProjectV2 shape)
    reef-wiki/              wiki as a git repository
    reef-actions/           workflow parsing, scheduling, runner protocols, artifacts, cache, OIDC
    reef-packages/          registries
    reef-search/            tantivy indexes
    reef-jobs/              job table, workers, cron
    reef-webhooks/          delivery, signing, redelivery
    reef-api/               axum: REST /api/v3, GraphQL /api/graphql, git, packages
    reef-web/               embedded Primer React app
  web/                       React, Vite, Primer
  bin/reef/                 CLI: serve, migrate, hook, admin
```

The binary runs everything: HTTP, SSH, workers, and the git hook entry point (`reef hook pre-receive`). Migrations follow the POC's rule: numbered, forward-only, applied at start under an advisory lock, never edited after shipping.

## 9. Decisions (27 September 2026)

Rohit settled the open questions:

1. **Primer plus a small theme layer.** Code hosting, Actions, issues, pull requests, and projects look exactly like GitHub. The theme layer is branding: logo, accent, fonts, and customisation hooks.
2. **GitHub-native runner protocol.** Reef speaks the protocol of `actions/runner` so stock GitHub runners work. The Gitea protocol is not a phase.
3. **Flat, GitHub-style tenancy.** Users, organizations, teams. No nesting.
4. **One binary, multi-host.** Every Reef process is stateless apart from the shared database and shared storage, so any number of hosts can run behind a load balancer. Repositories, LFS, packages, and artifacts live on shared storage (a shared filesystem or an S3-compatible store). Sessions, jobs, and locks live in the database. Role flags (`--role web,ssh,worker`) let a deployment split roles later.
5. **Native auth is required** alongside OIDC: username and password with argon2, TOTP, passkeys, sessions, personal access tokens, and the OAuth device flow for `gh`.

## 9a. Original open questions (for the record)

1. **Primer versus a bespoke design system** for Reef's chrome (section 4). Recommended: Primer with a small theme layer.
2. **Runner phase one protocol**: Gitea's Connect-RPC (fast, reuse Forgejo runner) or GitHub's native protocol first (slower, stock runners work immediately). Recommended: Gitea's first, GitHub's second.
3. **Tenancy model**: Reef organizations are flat like GitHub's, or nested like a cloud tenancy tree. GitHub compatibility argues for flat organizations with teams, and an optional link from a Reef organization to a group at the identity provider for single sign-on.
4. **Single binary or split roles**: the blueprint says one binary per role. Reef can start as one binary with `--role` flags (web, ssh, worker) so it can split later without a rewrite.

## 10. Sources

Git internals: gitoxide crate-status.md and discussion #1299; git2-rs issue #591; Gitea `routers/web/repo/githttp.go`; Forgejo PR #10397; Gitaly upload-pack design document; russh; GitArena; w4/rgit; josh-project/josh; git-lfs batch API; rudolfs; git-lfs-api crate.

GitHub compatibility: github/rest-api-description; GitHub changelog for API version 2026-03-10; cli/cli `internal/ghinstance/host.go`, `internal/featuredetection`, `api/queries_pr.go`, `api/queries_repo.go`; Terraform provider `provider.go`; Renovate GitHub platform docs; git-credential-manager autodetect docs.

Actions: actions/runner source (ConfigurationManager, RunnerDotcomServer, MessageListener, NodeScriptActionHandler); ChristopherHX/runner.server and github-act-runner; gitea/actions-proto-def; Gitea Actions design and comparison docs; Forgejo v15 release notes and OIDC docs; zizmor crates; gha-cache-oxide.

Packages: Cargo registry web API and index docs; RFC 3139; OCI distribution spec; oci-spec; trow; Gitea and Forgejo package docs; PEP 503.

Stack: axum changelog; sqlx 0.9 changelog; sea-orm 2.0 announcement; sqlx issues #964 and discussion #1094; tantivy; apalis; openidconnect-rs; webauthn-rs; tower-sessions; ssh-key; rPGP; sequoia; @primer/react on npm and discussion #7086; GitHub engineering post on CSS Modules; comrak; two-face; imara-diff; similar; rust-embed; axum-vite; tracing-opentelemetry.
