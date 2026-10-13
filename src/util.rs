use std::path::Path as FsPath;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn abs_url(public: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if public.is_empty() {
        return format!("/{rel}");
    }
    let base = public.trim_end_matches('/');
    format!("{base}/{rel}")
}

pub fn sniff(ext: &str, data: &[u8]) -> bool {
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
