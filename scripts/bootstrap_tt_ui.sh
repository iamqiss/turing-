#!/usr/bin/env bash
# Recreate Turing Tarpit Dioxus UI tree (tt-ui + web + desktop stubs)
set -euo pipefail
ROOT="${1:-.}"
cd "$ROOT"
echo "Bootstrapping clients under $(pwd)"
mkdir -p clients/tt-ui/src/{api,components,pages} clients/web/src clients/desktop/src assets/brand
echo '  write clients/web/Cargo.toml'
cat > 'clients/web/Cargo.toml' << 'TT_EOF'

TT_EOF

echo '  write clients/web/src/main.rs'
cat > 'clients/web/src/main.rs' << 'TT_EOF'

TT_EOF

echo '  write clients/desktop/Cargo.toml'
cat > 'clients/desktop/Cargo.toml' << 'TT_EOF'

TT_EOF

echo '  write clients/desktop/src/main.rs'
cat > 'clients/desktop/src/main.rs' << 'TT_EOF'

TT_EOF

echo '  write assets/brand/tt-mark.svg'
cat > 'assets/brand/tt-mark.svg' << 'TT_EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 64 64">
  <rect x="0" y="0" width="64" height="64" rx="14" fill="#0a0a0a"/>
  <rect x="4" y="4" width="56" height="56" rx="11" fill="#ff3c00"/>
  <path d="M13 21.5 C13 21.5 50.5 20.5 51 21.5" stroke="#fafaf7" stroke-width="7.2" stroke-linecap="round" fill="none"/>
  <path d="M23.5 21.5 L23.5 47.5" stroke="#fafaf7" stroke-width="6.4" stroke-linecap="round"/>
  <path d="M40.5 21.5 L40.5 47.5" stroke="#fafaf7" stroke-width="6.4" stroke-linecap="round"/>
</svg>
TT_EOF


# Ensure workspace members list clients (idempotent)
if ! grep -q 'clients/tt-ui' Cargo.toml 2>/dev/null; then
  echo "NOTE: add clients/tt-ui, clients/web, clients/desktop to [workspace].members in Cargo.toml"
fi

echo "Done. Tree:"
find clients/tt-ui clients/web clients/desktop assets/brand -type f 2>/dev/null | sort
echo
echo "Next: cargo check -p tt-ui   (or dx serve from clients/web)"

