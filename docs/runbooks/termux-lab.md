# Termux lab runbook

## Packages

```bash
pkg update && pkg upgrade -y
pkg install postgresql redis git rust clang make pkg-config openssl -y
```

## Postgres

```bash
mkdir -p $PREFIX/var/lib/postgresql
initdb -D $PREFIX/var/lib/postgresql   # once
pg_ctl -D $PREFIX/var/lib/postgresql -l $PREFIX/var/log/postgresql.log start
createuser -s $(whoami) 2>/dev/null || true
createdb turing_tarpit 2>/dev/null || true
```

## Redis

```bash
redis-server --daemonize yes
redis-cli ping
```

## Env

```bash
cd ~/tt
export DATABASE_URL="postgres://$(whoami)@localhost/turing_tarpit"
export REDIS_URL="redis://127.0.0.1:6379"
export TT_SESSION_KEY="$(openssl rand -hex 32)"
export TT_SECRETS_KEY="$(openssl rand -hex 32)"
export TT_KMS_PROVIDER=local
export TT_OBJECT_STORE=local
export TT_MEDIA_ROOT="$HOME/tt-data/media"
export TT_GIT_REPOS_ROOT="$HOME/tt-data/repos"
mkdir -p "$TT_MEDIA_ROOT" "$TT_GIT_REPOS_ROOT"
export CARGO_BUILD_JOBS=1   # if linker OOM
```

Persist in `~/tt/.env` and `source` it after reboot.

## Run

```bash
cargo run -p tt-server
curl -s http://127.0.0.1:8080/healthz
```

Migrations apply on boot (001–021).

## After phone reboot

```bash
pg_ctl -D $PREFIX/var/lib/postgresql -l $PREFIX/var/log/postgresql.log start
redis-server --daemonize yes
cd ~/tt && source .env
cargo run -p tt-server
```

## Notes

- ClickHouse / Meilisearch / ClamAV / tt-kms optional in lab  
- Ephemeral `SESSION_KEY` → sessions die on restart; set `TT_SESSION_KEY`  
- New `TT_SECRETS_KEY` cannot decrypt old Scarlet ciphertext  
