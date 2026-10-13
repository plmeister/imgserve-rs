# imgserve (Rust)

Minimal self-hosted image/SVG upload + serve service. Written in Rust with axum + SQLite (sqlx).

## Features

- `POST /api/upload` with Bearer token, multipart `file`. Returns `{hash,url,thumb}`.
- `GET /v/{name}?t=<read_token>` when read token configured.
- SVG stored as `.svg.gz` and served with `Content-Encoding: gzip`.
- WebP thumbnails for rasters.
- Content-addressed by SHA256 (first 32 hex).
- Configurable public URL via `IMG_PUBLIC_URL`.

## Environment

| Var | Default | Description |
|---|---|---|
| `IMG_TOKEN` | `""` | Write token |
| `IMG_READ_TOKEN` | `""` | Read token (required for `/v/*` if set) |
| `IMG_TTL_DAYS` | `30` | TTL for cleanup (WIP in this build) |
| `IMG_QUOTA_MB` | `1024` | Quota for cleanup (WIP) |
| `IMG_MAX_MB` | `20` | Max upload size |
| `IMG_REJECT_SVG` | `0` | Reject SVG if `1` |
| `IMG_THUMB_WIDTH` | `1024` | Thumb width |
| `IMG_PUBLIC_URL` | `""` | Public base (e.g. `https://example.com/img`) |
| `DATA_DIR` | `/data` | Storage dir |
| `BIND` | `0.0.0.0:8000` | Bind address |
