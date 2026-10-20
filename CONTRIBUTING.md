# Contributing to Reef

Thanks for helping. Reef is a Rust workspace with a React web UI; `make help` lists every
developer target and the README covers the quick start.

## Before you open a pull request

- `make check` runs what CI runs: `cargo fmt --check`, `clippy -D warnings`, the Rust tests
  and the web typecheck. Set `REEF_TEST_DATABASE_URL=postgres://...` to run the data layer
  against PostgreSQL as well.
- Keep the GitHub API compatibility promise: `gh`, octokit and the stock `actions/runner`
  must keep working unchanged. When adding an endpoint, mirror GitHub's shapes and names.
- Configuration keys get a default, an environment twin (automatic), a line in
  `deploy/reef.conf.example` and a row in `docs/configuration.md`.
- Database changes are expand-and-contract migrations under `crates/reef-db/migrations`:
  binary N must run against schema N+1 (see `DEPLOY.md`, "Start, migrate, upgrade").

## Reporting bugs and proposing features

Open an issue with the version (`reef version`), the database backend, and what you expected.
Security problems go through private vulnerability reporting instead (see `SECURITY.md`).
`TODO.md` lists what is already planned.

## License

By contributing you agree that your contributions are licensed under the MIT license that
covers the project.
