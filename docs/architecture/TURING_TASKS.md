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

## Dogfooding Turing Tarpit

This repo ships its own pipelines under **`.tt/pipelines/`**:

| File | Purpose |
|------|---------|
| `ci.yml` | fmt + clippy + unit tests on push/PR to main |
| `ship.yml` | release gate: workspace release build + scarlet scan |

Register + smoke against a local/control-plane server:

```bash
export TT_BASE=http://127.0.0.1:8080
export TT_TOKEN=<bearer>
export TT_WS=<workspace-uuid>
export TT_REPO=turing-tarpit
export TT_ORG_SLUG=<org>   # optional usage check

./scripts/register_dogfood_pipelines.sh
./scripts/smoke_tasks.sh
```

When runners leave the control-plane stub, the same YAML runs on your
Latitude/OVH metals via `tt-node` — no GitHub Actions required.
