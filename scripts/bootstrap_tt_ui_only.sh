#!/usr/bin/env bash
set -euo pipefail
cd "${1:-.}"
mkdir -p clients/tt-ui/src/api clients/tt-ui/src/components clients/tt-ui/src/pages
echo
echo 'Verify:'
ls -la clients/tt-ui/src/logo.rs clients/tt-ui/src/theme.rs clients/tt-ui/Cargo.toml
find clients/tt-ui -type f | sort
