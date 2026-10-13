use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
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
    #[arg(long, default_value = "127.0.0.1:8000")]
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
