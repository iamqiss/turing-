# Media — object store, thumbnails, virus scan

## Object stores (`ObjectStore` trait)

| `TT_OBJECT_STORE` | Backend |
|-------------------|---------|
| `local` (default) | Filesystem under `TT_MEDIA_ROOT` |
| `s3` / `rustfs` / `minio` | S3-compatible SigV4 (path-style) |

```bash
# Local (Termux / single node)
export TT_OBJECT_STORE=local
export TT_MEDIA_ROOT=$HOME/tt-data/media

# RustFS / MinIO on metal
export TT_OBJECT_STORE=rustfs
export TT_S3_ENDPOINT=http://10.0.0.8:9000
export TT_S3_ACCESS_KEY=...
export TT_S3_SECRET_KEY=...
export TT_S3_BUCKET=turing-tarpit
export TT_S3_REGION=us-east-1
```

Create the bucket once in RustFS/MinIO before first upload.

## Thumbnails

On image upload (jpeg/png/webp/gif), `make_thumbnail` produces a max-edge JPEG stored as
`{storage_key}.thumb.jpg`. Served at:

```
GET /api/media/:id/thumb
```

JSON responses include `thumb_url`, `width`, `height`.

## Virus scan (ClamAV)

| `TT_CLAMD_ADDR` | Behaviour |
|-----------------|------------|
| empty | `scan_status = skipped` (lab) |
| `127.0.0.1:3310` | clamd `zINSTREAM` — reject infected uploads |

```bash
# Debian/metal
clamd
export TT_CLAMD_ADDR=127.0.0.1:3310
```

Infected files never hit object storage; API returns 400.

## Schema

Migration `021_media_thumbs_scan.sql` adds `thumb_key`, `width`, `height`, `scan_status`, `scanned_at`.
