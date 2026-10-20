# Metrics

`GET /metrics` serves Prometheus text on the primary listener, or only on `[metrics].listen`
when that is set (`docs/configuration.md`). It sits outside authentication, the request
timeout, body limits and rate limiting, logs at `debug`, and is counted in the request series
like any route. Scrape every instance: the
request series are per instance, the gauges read the shared database and so agree across
instances.

## Series

| Series | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `reef_build_info` | gauge (1) | `version`, `commit` | The build: `version` `v0.1.0`, `commit` the git commit (`-dirty` when built from a modified tree). |
| `reef_start_time_seconds` | gauge | | Unix time the process started. |
| `reef_http_requests_total` | counter | `method`, `route`, `status` | Requests by matched route pattern (`unmatched` when none) and status code; `method` is a standard verb or `other`. Health, readiness and metrics requests included. |
| `reef_http_request_duration_seconds` | histogram | `method`, `route` | Request duration; Prometheus' default buckets. |
| `reef_http_rate_limited_total` | counter | | API requests refused with 429. |
| `reef_repositories_total` | gauge | | Repositories. |
| `reef_users_total` | gauge | | User accounts. |
| `reef_organizations_total` | gauge | | Organizations. |
| `reef_actions_jobs` | gauge | `state` = `queued`, `running`, `completed`, `failed` | Workflow jobs by state. `queued` includes jobs waiting on `needs`; `failed` is the subset of `completed` whose conclusion is `failure` or `timed_out`. |
| `reef_actions_runners_online` | gauge | | Runners that polled in the last two minutes. |
| `reef_actions_logs_bytes` | gauge | | Bytes of job logs under `<data_dir>/actions/logs`. |
| `reef_actions_artifacts_bytes` | gauge | | Bytes of artifacts under `<data_dir>/actions/artifacts`. |
| `reef_actions_runs_purged_total` | counter | | Runs removed by the retention job (this instance). |
| `reef_webhook_deliveries_pending` | gauge | | Deliveries queued or waiting for a retry. |
| `reef_webhook_deliveries_total` | counter | `result` = `delivered`, `retried`, `failed` | Delivery attempts made by this instance. |
| `reef_mirror_syncs_total` | counter | `result` = `ok`, `error` | Pull mirror syncs run by this instance. |
| `process_*` | | | CPU, memory, open fds, threads (Linux only, from the `prometheus` crate). |

Rules:

- `route` is the router's matched pattern (`/api/v3/repos/{owner}/{repo}`), never the
  request path, so cardinality is bounded by the number of routes. Requests that match no
  route (the SPA fallback, 404s) are labelled `unmatched`. `method` is one of the nine standard
  methods or `other`. The health, readiness and metrics endpoints are counted too.
- The database gauges (`reef_repositories_total` .. `reef_webhook_deliveries_pending`) are
  refreshed every 30 s by every instance.
- The byte gauges walk the shared `actions/` tree and are refreshed by the retention job
  (at start and hourly), because the walk is the expensive part on a network filesystem.
- Counters are per instance and reset when the process restarts; use `rate()` / `increase()`.

## Useful queries

```promql
# request rate and p95 latency by route
sum by (route) (rate(reef_http_requests_total[5m]))
histogram_quantile(0.95, sum by (le, route) (rate(reef_http_request_duration_seconds_bucket[5m])))

# errors
sum(rate(reef_http_requests_total{status=~"5.."}[5m])) / sum(rate(reef_http_requests_total[5m]))

# Actions backlog: jobs waiting with no runner to take them
reef_actions_jobs{state="queued"} > 0 and reef_actions_runners_online == 0

# webhooks piling up (deliveries pending for a while)
reef_webhook_deliveries_pending > 100

# mirror failures
increase(reef_mirror_syncs_total{result="error"}[1h]) > 0
```

## Alerts worth having

- `up == 0` on `/metrics`, or `/readyz` failing (the load balancer's own check).
- `reef_actions_runners_online == 0` while `reef_actions_jobs{state="queued"} > 0` for 10 min.
- `rate(reef_http_requests_total{status=~"5.."}[5m]) > 0` sustained.
- `reef_actions_logs_bytes + reef_actions_artifacts_bytes` approaching the volume size:
  lower `[actions] retention` (see `DEPLOY.md`, section 6).
