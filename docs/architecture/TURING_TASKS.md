# Turing Tasks (`tt-pipelines`)

CI pipelines under **`.tt/pipelines/*.yml`**, billed as **`tasks.minutes`**.

## Crate layout

```
crates/tt-pipelines/
  src/
    lib.rs       # re-exports
    error.rs
    yaml.rs      # parse + trigger match
    store.rs     # Postgres TaskStore
    service.rs   # TaskService (register / dispatch / bill)
```

## Example pipeline

```yaml
# .tt/pipelines/ci.yml
name: ci
on:
  push:
    branches: [main]
  workflow_dispatch:
jobs:
  test:
    runs-on: tt-linux-latest
    steps:
      - run: cargo test
```

## Billing

| Plan | Included min/mo | Overage |
|------|-----------------|--------|
| free | 500 | $0.008/min |
| team | 3_000 | same |
| business | 15_000 | same |
| enterprise | 100_000 | same |

v1 stub: **1 billable minute per step**. Real runners report wall time later.

## API

```http
POST /api/workspaces/:ws/repos/:repo/tasks/pipelines
GET  /api/workspaces/:ws/repos/:repo/tasks/pipelines
POST /api/workspaces/:ws/repos/:repo/tasks/runs
GET  /api/workspaces/:ws/repos/:repo/tasks/runs
GET  /api/tasks/runs/:id
GET  /api/orgs/:slug/tasks/usage
```

Schema: `migrations/postgres/028_turing_tasks.sql`
