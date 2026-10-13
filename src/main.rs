use axum::{
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use imgserve::config::{AppState, Args};
use imgserve::db::init_db;
use imgserve::handlers::{cleanup::cleanup, upload::upload, view::view};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let args = Arc::new(args);

    let data_dir = PathBuf::from(&args.data_dir);
    let orig_dir = data_dir.join("orig");
    fs::create_dir_all(&orig_dir)?;

    let db_path = data_dir.join("imgserve.db");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
        .await?;

    init_db(&pool).await?;

    let state = AppState {
        args: args.clone(),
        pool,
        orig_dir,
    };

    let state_clone = state.clone();
    tokio::spawn(async move {
        let _ = cleanup(&state_clone).await;
        loop {
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            let _ = cleanup(&state_clone).await;
        }
    });

    let app = Router::new()
        .route("/", get(|| async { Json(serde_json::json!({"ok": true})) }))
        .route("/api/upload", post(upload))
        .route("/v/{name}", get(view))
        .layer(axum::extract::DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(100 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!("listening on {}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
