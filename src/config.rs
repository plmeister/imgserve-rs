use clap::Parser;
use std::sync::Arc;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "imgserve (Rust)", long_about = None)]
pub struct Args {
    #[arg(long, env = "IMG_TOKEN", default_value = "")]
    pub token: String,
    #[arg(long, env = "IMG_READ_TOKEN", default_value = "")]
    pub read_token: String,
    #[arg(long, env = "IMG_TTL_DAYS", default_value = "30")]
    pub ttl_days: f64,
    #[arg(long, env = "IMG_QUOTA_MB", default_value = "1024")]
    pub quota_mb: f64,
    #[arg(long, env = "IMG_MAX_MB", default_value = "20")]
    pub max_mb: f64,
    #[arg(long, env = "IMG_REJECT_SVG", default_value = "0")]
    pub reject_svg: i32,
    #[arg(long, env = "IMG_PUBLIC_URL", env = "IMG_BASE_URL", default_value = "")]
    pub public_url: String,
    #[arg(long, env = "DATA_DIR", default_value = "/data")]
    pub data_dir: String,
    #[arg(long, default_value = "0.0.0.0:8000")]
    pub bind: String,
}

#[derive(Clone)]
pub struct AppState {
    pub args: Arc<Args>,
    pub pool: sqlx::SqlitePool,
    pub orig_dir: std::path::PathBuf,
}
