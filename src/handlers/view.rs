use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use serde::Deserialize;
use std::path::PathBuf;

use crate::config::AppState;
use crate::util::now_secs;

#[derive(Deserialize)]
pub struct ViewQuery {
    t: Option<String>,
}

pub async fn view(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let read_token = &state.args.read_token;
    if !read_token.is_empty() {
        match q.t.as_ref() {
            Some(t) if t == read_token => {}
            _ => return Err((StatusCode::FORBIDDEN, "forbidden".to_string())),
        }
    }
    let name = name.trim();
    let re = regex::Regex::new(r"^([0-9a-f]{8,})(\.[a-z0-9]+)$").unwrap();
    let caps = match re.captures(name) {
        Some(c) => c,
        None => return Err((StatusCode::NOT_FOUND, "not found".to_string())),
    };
    let hash = caps.get(1).unwrap().as_str().to_string();
    let ext = caps.get(2).unwrap().as_str().to_string();
    match ext.as_str() {
        ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".bmp" | ".tif" | ".tiff" | ".avif" | ".ico" | ".svg" => {}
        _ => return Err((StatusCode::NOT_FOUND, "not found".to_string())),
    }
    let mut file_path: PathBuf = state.orig_dir.join(name);
    if ext == ".svg" {
        let gz_path = state.orig_dir.join(format!("{hash}{ext}.gz"));
        if gz_path.exists() {
            file_path = gz_path;
        }
    }
    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, "not found".to_string()));
    }
    let file = tokio::fs::File::open(&file_path).await.map_err(|_| (StatusCode::NOT_FOUND, "not found".to_string()))?;
    let stream = tokio_util::io::ReaderStream::new(file);
    let mut resp = Response::new(axum::body::Body::from_stream(stream));
    if ext == ".svg" {
        resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
        resp.headers_mut().insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        resp.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:"));
    } else {
        let mime = mime_guess::from_ext(ext.trim_start_matches('.')).first_or_octet_stream();
        resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    }
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=2592000"));
    resp.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    let _ = sqlx::query("UPDATE images SET last_access = ? WHERE hash = ?").bind(now_secs()).bind(&hash).execute(&state.pool).await;
    Ok(resp)
}
