use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use image::ImageFormat;
use libflate::gzip;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path as FsPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "imgserve (Rust)", long_about = None)]
struct Args {
    #[arg(long, env = "IMG_TOKEN", default_value = "")]
    token: String,
    #[arg(long, env = "IMG_READ_TOKEN", default_value = "")]
    read_token: String,
    #[arg(long, env = "IMG_TTL_DAYS", default_value = "30")]
    ttl_days: f64,
    #[arg(long, env = "IMG_QUOTA_MB", default_value = "1024")]
    quota_mb: f64,
    #[arg(long, env = "IMG_MAX_MB", default_value = "20")]
    max_mb: f64,
    #[arg(long, env = "IMG_REJECT_SVG", default_value = "0")]
    reject_svg: i32,
    #[arg(long, env = "IMG_THUMB_WIDTH", default_value = "1024")]
    thumb_width: u32,
    #[arg(long, env = "IMG_PUBLIC_URL", env = "IMG_BASE_URL", default_value = "")]
    public_url: String,
    #[arg(long, env = "DATA_DIR", default_value = "/data")]
    data_dir: String,
    #[arg(long, default_value = "0.0.0.0:8000")]
    bind: String,
}

#[derive(Clone)]
struct AppState {
    args: Arc<Args>,
    pool: SqlitePool,
    orig_dir: PathBuf,
    thumbs_dir: PathBuf,
}

#[derive(Serialize)]
struct UploadResp {
    hash: String,
    url: String,
    thumb: String,
}

#[derive(Deserialize)]
struct ViewQuery {
    t: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Empty {}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn abs_url(public: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if public.is_empty() {
        return format!("/{rel}");
    }
    let base = public.trim_end_matches('/');
    format!("{base}/{rel}")
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let args = Arc::new(args);

    let data_dir = PathBuf::from(&args.data_dir);
    let orig_dir = data_dir.join("orig");
    let thumbs_dir = data_dir.join("thumbs");
    fs::create_dir_all(&orig_dir)?;
    fs::create_dir_all(&thumbs_dir)?;

    let db_path = data_dir.join("imgserve.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
        .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS images (
                hash TEXT PRIMARY KEY,
                ext TEXT NOT NULL,
                size INTEGER NOT NULL,
                svg INTEGER NOT NULL DEFAULT 0,
                created_at REAL NOT NULL,
                last_access REAL NOT NULL
            )"#,
    )
    .execute(&pool)
    .await?;

    sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;

    let state = AppState {
        args: args.clone(),
        pool,
        orig_dir,
        thumbs_dir,
    };

    let app = Router::new()
        .route("/", get(|| async { Json(serde_json::json!({"ok": true})) }))
        .route("/api/upload", post(upload))
        .route("/v/{name}", get(view))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(100 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!("listening on {}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

fn sniff(ext: &str, data: &[u8]) -> bool {
    match ext {
        ".png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        ".jpg" | ".jpeg" => data.starts_with(b"\xff\xd8\xff"),
        ".gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        ".webp" => data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP",
        ".bmp" => data.len() > 2 && &data[0..2] == b"BM",
        ".tif" | ".tiff" => data.starts_with(b"II*\x00") || data.starts_with(b"MM\x00*"),
        ".avif" => data.len() > 12 && &data[4..8] == b"ftyp" && (&data[8..11] == b"avif" || &data[8..11] == b"heic"),
        ".ico" => data.len() > 4 && (data.starts_with(b"\x00\x00\x01\x00") || data.starts_with(b"\x00\x00\x02\x00")),
        ".svg" => true,
        _ => false,
    }
}

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResp>, (StatusCode, String)> {
    let token = state.args.token.clone();
    if !token.is_empty() {
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        match auth {
            Some(a) if a == format!("Bearer {token}") => {}
            _ => return Err((StatusCode::UNAUTHORIZED, "invalid or missing bearer token".to_string())),
        }
    }

    let mut file_name: Option<String> = None;
    let mut data: Vec<u8> = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = field.file_name().map(|s| s.to_string()).or(Some("upload".to_string()));
            data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?.to_vec();
            break;
        }
    }
    if data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no file field".to_string()));
    }

    let fname = file_name.unwrap_or_default();
    let mut ext = FsPath::new(&fname)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s.to_lowercase()))
        .unwrap_or_default();
    if ext == ".jpeg" {
        ext = ".jpg".to_string();
    }

    let mut data = data;
    let max_bytes = (state.args.max_mb.max(1.0)) as usize * 1024 * 1024;
    if data.len() > max_bytes {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, format!("image too large (max {}MB)", state.args.max_mb)));
    }
    if data.starts_with(b"\x1f\x8b") {
        let mut decoder = gzip::Decoder::new(&data[..]).map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid gzip: {e}")))?;
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid gzip: {e}")))?;
        data = decompressed;
        if data.len() > max_bytes {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, format!("image too large after decompression (max {}MB)", state.args.max_mb)));
        }
    }

    let mime_ok = match ext.as_str() {
        ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".bmp" | ".tif" | ".tiff" | ".avif" | ".ico" | ".svg" => true,
        _ => false,
    };
    if !mime_ok {
        return Err((StatusCode::BAD_REQUEST, format!("unsupported type: {}", ext)));
    }
    if state.args.reject_svg == 1 && ext == ".svg" {
        return Err((StatusCode::BAD_REQUEST, "svg uploads rejected by IMG_REJECT_SVG".to_string()));
    }
    if !sniff(ext.as_str(), &data) {
        return Err((StatusCode::BAD_REQUEST, format!("content does not match {}", ext)));
    }

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(&hasher.finalize()[..])[0..32].to_string();
    let is_svg = ext == ".svg";

    let now = now_secs();
    if is_svg {
        let mut encoder = libflate::gzip::Encoder::new(Vec::new()).unwrap();
        encoder.write_all(&data).unwrap();
        let stored = encoder.finish().into_result().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let path = state.orig_dir.join(format!("{hash}{ext}.gz"));
        if !path.exists() {
            fs::write(&path, &stored).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        let size = stored.len() as i64;
        let _ = sqlx::query("INSERT OR REPLACE INTO images (hash, ext, size, svg, created_at, last_access) VALUES (?, ?, ?, 1, ?, ?)")
            .bind(&hash)
            .bind(&ext)
            .bind(size)
            .bind(now)
            .bind(now)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let url_rel = format!("/v/{hash}{ext}");
        let thumb_rel = url_rel.clone();
        let resp = UploadResp {
            hash,
            url: abs_url(&state.args.public_url, &url_rel),
            thumb: abs_url(&state.args.public_url, &thumb_rel),
        };
        return Ok(Json(resp));
    } else {
        let path = state.orig_dir.join(format!("{hash}{ext}"));
        if !path.exists() {
            fs::write(&path, &data).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        let size = data.len() as i64;
        let _ = sqlx::query("INSERT OR REPLACE INTO images (hash, ext, size, svg, created_at, last_access) VALUES (?, ?, ?, 0, ?, ?)")
            .bind(&hash)
            .bind(&ext)
            .bind(size)
            .bind(now)
            .bind(now)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let thumb_path = state.thumbs_dir.join(format!("{hash}-sm.webp"));
        if !thumb_path.exists() {
            let _ = generate_thumb(&data, &thumb_path, state.args.thumb_width);
        }
        let url_rel = format!("/v/{hash}{ext}");
        let thumb_rel = format!("/v/{hash}-sm.webp");
        let resp = UploadResp {
            hash,
            url: abs_url(&state.args.public_url, &url_rel),
            thumb: abs_url(&state.args.public_url, &thumb_rel),
        };
        return Ok(Json(resp));
    }
}

fn generate_thumb(data: &[u8], out_path: &FsPath, max_w: u32) -> anyhow::Result<()> {
    let img = image::load_from_memory(data)?;
    let img = img.into_rgb8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Ok(());
    }
    let ratio = (max_w as f32) / (w as f32);
    if ratio >= 1.0 {
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::WebP)?;
        fs::write(out_path, &buf)?;
        return Ok(());
    }
    let nw = max_w;
    let nh = ((h as f32) * ratio) as u32;
    let thumb = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3);
    let mut buf = Vec::new();
    thumb.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::WebP)?;
    fs::write(out_path, &buf)?;
    Ok(())
}

async fn view(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let read_token = state.args.read_token.clone();
    if !read_token.is_empty() {
        match q.t.as_ref() {
            Some(t) if t == &read_token => {}
            _ => return Err((StatusCode::FORBIDDEN, "forbidden".to_string())),
        }
    }
    let name = name.trim();
    // parse: hash[-sm].ext
    let re = regex::Regex::new(r"^([0-9a-f]{8,})(?:-(sm))?(\.[a-z0-9]+)$").unwrap();
    let caps = match re.captures(name) {
        Some(c) => c,
        None => return Err((StatusCode::NOT_FOUND, "not found".to_string())),
    };
    let _hash = caps.get(1).unwrap().as_str().to_string();
    let variant = caps.get(2).map(|m| m.as_str()).unwrap_or("");
    let ext = caps.get(3).unwrap().as_str().to_string();
    match ext.as_str() {
        ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".bmp" | ".tif" | ".tiff" | ".avif" | ".ico" => {}
        ".svg" => {}
        _ => return Err((StatusCode::NOT_FOUND, "not found".to_string())),
    }
    if variant == "sm" {
        let path = state.thumbs_dir.join(name);
        if !path.exists() {
            return Err((StatusCode::NOT_FOUND, "not found".to_string()));
        }
        let bytes = tokio::fs::read(&path).await.map_err(|_| (StatusCode::NOT_FOUND, "not found".to_string()))?;
        let mut resp = Response::new(Body::from(bytes));
        resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/webp"));
        resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=2592000"));
        resp.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        return Ok(resp);
    }
    if ext == ".svg" {
        let gz_path = state.orig_dir.join(format!("{}{}.gz", _hash, ext));
        if gz_path.exists() {
            let bytes = tokio::fs::read(&gz_path).await.map_err(|_| (StatusCode::NOT_FOUND, "not found".to_string()))?;
            let mut resp = Response::new(Body::from(bytes));
            resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
            resp.headers_mut().insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=2592000"));
            resp.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            resp.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:"));
            let _ = sqlx::query("UPDATE images SET last_access = ? WHERE hash = ?").bind(now_secs()).bind(&_hash).execute(&state.pool).await;
            return Ok(resp);
        }
    }
    let path = state.orig_dir.join(name);
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "not found".to_string()));
    }
    let bytes = tokio::fs::read(&path).await.map_err(|_| (StatusCode::NOT_FOUND, "not found".to_string()))?;
    let mime = mime_guess::from_ext(ext.trim_start_matches('.')).first_or_octet_stream();
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=2592000"));
    resp.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let _ = sqlx::query("UPDATE images SET last_access = ? WHERE hash = ?").bind(now_secs()).bind(&_hash).execute(&state.pool).await;
    Ok(resp)
}
