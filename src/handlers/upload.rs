use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use libflate::gzip;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Path as FsPath;

use crate::config::AppState;
use crate::util::{abs_url, now_secs, sniff};

#[derive(Serialize)]
pub struct UploadResp {
    pub hash: String,
    pub url: String,
    pub thumb: String,
}

pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResp>, (StatusCode, String)> {
    let token = &state.args.token;
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
        let name = field.name().unwrap_or("");
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
        return Err((StatusCode::BAD_REQUEST, "svg uploads rejected".to_string()));
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
        return Ok(Json(UploadResp {
            hash,
            url: abs_url(&state.args.public_url, &url_rel),
            thumb: abs_url(&state.args.public_url, &url_rel),
        }));
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
        let url_rel = format!("/v/{hash}{ext}");
        return Ok(Json(UploadResp {
            hash,
            url: abs_url(&state.args.public_url, &url_rel),
            thumb: abs_url(&state.args.public_url, &url_rel),
        }));
    }
}
