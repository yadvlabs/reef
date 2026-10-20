# Reef backlog

Open items that are not scheduled yet, grouped by area. Items are removed when shipped;
see `docs/` for what exists today.

## UI / UX (GitHub parity)

- [ ] Follows and followers: Follow button and follower/following counts on user profiles, "N followers" on organizations, `/users/{login}/followers` and `/following` REST endpoints.
- [ ] Profile and organization READMEs: render `.github/profile/README.md` for organizations and `<login>/<login>` `README.md` for users on the Overview tab.
- [ ] Packages: container (OCI) registry first, then npm and cargo; Packages tab on repositories, organizations and profiles, and the sidebar card.
- [ ] Discussions tab (categories, threads, answers, polls).
- [ ] Code search (`/search/code` returns empty); commit search; symbol search in files.
- [ ] Issues: saved filters, keyboard shortcuts (`c`, `j/k`, `x`), reactions on list rows, issue forms (`.github/ISSUE_TEMPLATE/*.yml`), sub-issues, issue types.
- [ ] Pull requests: suggested changes ("Apply suggestion"), review requests via CODEOWNERS, required reviews and status checks enforced by branch protection UI, merge queue.
- [ ] Notifications: per-repository subscription settings page, email delivery, digest, "Custom" watch options for releases/discussions/security alerts.
- [ ] Mobile layouts below ~900px for the project board, insights charts and the file table.
- [ ] Releases: asset uploads (binaries), auto-generated release notes, draft release editing of assets.
- [ ] Wiki: page history diff view, sidebar/footer pages, wiki search.
- [ ] Global search page facets (language, stars, updated) and sort.
- [ ] Repository settings: rulesets (beyond classic branch protection), deploy keys UI, secrets scanning toggles, Pages.
- [ ] Organization: teams with nested teams and team-level repo permissions UI, member privileges page, verified domains (the org header shows no verified badge until domains exist), audit log page.
- [ ] Admin console: org creation on behalf of users, repo transfer, site-wide announcement scheduling, background job status.
- [ ] Contribution graph: count PR reviews and discussions, "private contributions" toggle, streak tooltips, shareable image.


## Actions

- [ ] Artifact upload/download and cache services for `actions/upload-artifact` and `actions/cache`.
- [ ] Reusable workflows (`workflow_call`) and composite actions hosted in Reef repositories.
- [ ] Scheduled (`cron`) triggers.
- [ ] Ephemeral runner pools; runner groups and org-level runners UI.
- [ ] Job summaries (`GITHUB_STEP_SUMMARY`), annotations, log search, re-run failed jobs only.
- [ ] `/debug/pprof`-style profiling on the metrics listener.
- [ ] Actions OIDC tokens (`id-token: write`) for cloud deployments.

## Identity

- [ ] Organization membership from a provider `groups` claim in oidc mode (create, add, remove, roles).
- [ ] Optional OIDC sign-in alongside password accounts in standalone mode.

## Platform

- [ ] Outbound allow-list per host name for webhooks and mirrors (today it is a single global `network.allow_private_targets` switch).
- [ ] Redirects from old user/organization/repository names after a rename.

- [ ] Email verification for addresses added under Settings → Emails (addresses are accepted unverified today).
- [ ] Repository transfer between owners (rename/redirects), and LFS object garbage collection for deleted history.
- [ ] Object-storage backend (S3-compatible) for Actions artifacts, caches and LFS objects instead of `data_dir`.

- [ ] Postgres-backed contribution and statistics tables (today insights walk each repository's commit index; fine for dev scale, not fleet scale).
- [ ] Repository transfer and rename redirects; repository archival banner behaviour for API writes.
- [ ] Rate-limit buckets shared between instances (today they are in memory per instance, so N instances allow up to N times `[limits]`).
- [ ] Backup/restore CLI (`reef backup`) covering the database, repositories and Actions logs.
