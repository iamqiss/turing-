#!/usr/bin/env bash
# Smoke-test Turing Tasks against a running tt-server.
# Usage:
#   export TT_BASE=http://127.0.0.1:8080
#   export TT_TOKEN=...          # Bearer from login
#   export TT_WS=...             # workspace uuid
#   export TT_REPO=turing-tarpit # repo name
#   ./scripts/smoke_tasks.sh

set -euo pipefail

BASE="${TT_BASE:-http://127.0.0.1:8080}"
TOKEN="${TT_TOKEN:?set TT_TOKEN (Bearer access token)}"
WS="${TT_WS:?set TT_WS (workspace id)}"
REPO="${TT_REPO:-turing-tarpit}"

auth=(-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json")

echo "==> register pipeline ci"
CI_YAML=$(cat .tt/pipelines/ci.yml)
# JSON-escape via python for safe body
BODY=$(python3 - << PY
import json, pathlib
yaml = pathlib.Path(".tt/pipelines/ci.yml").read_text()
print(json.dumps({"name": "ci", "yaml": yaml, "path": ".tt/pipelines/ci.yml"}))
PY
)
curl -sf "${auth[@]}" -d "$BODY" \
  "$BASE/api/workspaces/$WS/repos/$REPO/tasks/pipelines" | tee /tmp/tt_pipe.json
echo

echo "==> list pipelines"
curl -sf "${auth[@]}" \
  "$BASE/api/workspaces/$WS/repos/$REPO/tasks/pipelines" | tee /tmp/tt_pipes.json
echo

echo "==> dispatch manual run"
curl -sf "${auth[@]}" -d '{"pipeline":"ci","trigger":"manual","git_ref":"refs/heads/main"}' \
  "$BASE/api/workspaces/$WS/repos/$REPO/tasks/runs" | tee /tmp/tt_run.json
echo

RUN_ID=$(python3 -c "import json; print(json.load(open('/tmp/tt_run.json'))['id'])")
echo "==> get run $RUN_ID"
curl -sf "${auth[@]}" "$BASE/api/tasks/runs/$RUN_ID" | tee /tmp/tt_run_detail.json
echo

echo "==> list runs"
curl -sf "${auth[@]}" \
  "$BASE/api/workspaces/$WS/repos/$REPO/tasks/runs?limit=5"
echo

if [[ -n "${TT_ORG_SLUG:-}" ]]; then
  echo "==> org tasks usage ($TT_ORG_SLUG)"
  curl -sf "${auth[@]}" "$BASE/api/orgs/$TT_ORG_SLUG/tasks/usage"
  echo
fi

echo
echo "OK — Turing Tasks smoke passed"
echo "  pipeline: ci (.tt/pipelines/ci.yml)"
echo "  run:      $RUN_ID"
