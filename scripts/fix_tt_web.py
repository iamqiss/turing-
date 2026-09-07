#!/usr/bin/env python3
"""Create clients/web binary crate. Run from repo root:
   python3 scripts/fix_tt_web.py
"""
from pathlib import Path
import sys

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".")

FILES = {
    "clients/web/Cargo.toml": """[package]
name = "tt-web"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
publish.workspace = true
description = "Turing Tarpit web client (Dioxus)"

[dependencies]
tt-ui = { path = "../tt-ui" }
dioxus = { version = "0.6", features = ["web", "router"] }
serde = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
""",
    "clients/web/src/main.rs": """//! Turing Tarpit — web client
use tt_ui::App;

fn main() {
    dioxus::launch(App);
}
""",
    "clients/web/Dioxus.toml": """[application]
name = "tt-web"
default_platform = "web"

[web.app]
title = "Turing Tarpit"

[web.watcher]
reload_html = true
""",
}

for rel, content in FILES.items():
    p = ROOT / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content)
    print("write", p)

# Ensure workspace member
cargo = ROOT / "Cargo.toml"
if cargo.exists():
    t = cargo.read_text()
    if "clients/web" not in t:
        print("NOTE: add \"clients/web\" to [workspace].members in Cargo.toml (with trailing commas)")
    else:
        print("workspace already lists clients/web")

print("done")
print("  cargo install dioxus-cli --version 0.6.3 --locked --force")
print("  cd clients/web && dx serve")
