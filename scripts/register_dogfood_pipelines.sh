#!/usr/bin/env bash
set -euo pipefail
BASE="${TT_BASE:-http://127.0.0.1:8080}"
TOKEN="${TT_TOKEN:?set TT_TOKEN}"
WS="${TT_WS:?set TT_WS}"
REPO="${TT_REPO:-turing-tarpit}"
auth=(-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json")

for name in ci ship; do
  f=".tt/pipelines/${name}.yml"
  body=$(python3 -c "import json,pathlib; print(json.dumps({'name':'$name','yaml':pathlib.Path('$f').read_text(),'path':'$f'}))")
  echo "register $name"
  curl -sf "${auth[@]}" -d "$body" \
    "$BASE/api/workspaces/$WS/repos/$REPO/tasks/pipelines"
  echo
done
echo "registered ci + ship"
