# Turing Tarpit development commands

default:
    @just --list

# Start infrastructure
infra:
    docker compose up -d

infra-down:
    docker compose down

# Run server
server:
    cargo run -p tt-server

# Check workspace
check:
    cargo check --workspace

# Format
fmt:
    cargo fmt --all

# Clippy
clippy:
    cargo clippy --workspace -- -D warnings

# Generate a SESSION_KEY
session-key:
    @openssl rand -hex 32

# Proto generation (later)
proto:
    @echo "proto generation not yet wired"

migrate:
    @echo "migrations run automatically on server start"
