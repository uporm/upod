use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use axum::extract::{Multipart, Query};
use axum::http::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::models::filesystem::FileMetadata;

use super::file_operations::{apply_permission, map_file_error};

#[derive(Debug, Deserialize)]
pub(crate) struct DownloadReq {
    path: String,
}

pub(crate) async fn upload_file(mut multipart: Multipart) -> impl IntoResponse {
    let mut metadata_entries = Vec::<FileMetadata>::new();
    let mut file_entries = Vec::<Vec<u8>>::new();

    loop {
        let next = multipart.next_field().await;
        let field = match next {
            Ok(value) => value,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "invalid multipart payload").into_response();
            }
        };
        let Some(field) = field else {
            break;
        };
        let Some(name) = field.name().map(|v| v.to_string()) else {
            continue;
        };
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid multipart field").into_response(),
        };

        if name == "metadata" {
            let metadata: FileMetadata = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) => {
                    return (StatusCode::BAD_REQUEST, "invalid metadata json").into_response();
                }
            };
            metadata_entries.push(metadata);
        } else if name == "file" {
            file_entries.push(bytes.to_vec());
        }
    }

    if metadata_entries.is_empty() {
        return (StatusCode::BAD_REQUEST, "metadata file is missing").into_response();
    }
    if file_entries.is_empty() {
        return (StatusCode::BAD_REQUEST, "file is missing").into_response();
    }
    if metadata_entries.len() != file_entries.len() {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "metadata and file count mismatch: {} vs {}",
                metadata_entries.len(),
                file_entries.len()
            ),
        )
            .into_response();
    }

    for (metadata, content) in metadata_entries.iter().zip(file_entries.iter()) {
        if metadata.path.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, "metadata path is empty").into_response();
        }

        let target = PathBuf::from(&metadata.path);
        let target_dir = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if let Err(error) = fs::create_dir_all(&target_dir) {
            return map_file_error(error);
        }
        if let Err(error) = fs::write(&target, content) {
            return map_file_error(error);
        }
        if let Err(error) = apply_permission(metadata.path.as_str(), &metadata.permission) {
            return map_file_error(error);
        }
    }

    StatusCode::OK.into_response()
}

pub(crate) async fn download_file(
    Query(query): Query<DownloadReq>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if query.path.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "missing query parameter 'path'").into_response();
    }

    let file_path = PathBuf::from(&query.path);
    let metadata = match fs::metadata(&file_path) {
        Ok(value) => value,
        Err(error) => return map_file_error(error),
    };
    if !metadata.is_file() {
        return (StatusCode::BAD_REQUEST, "path is not a file").into_response();
    }

    let filename = file_path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("download.bin");
    let file_size = metadata.len();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename={filename}")) {
        response_headers.insert(CONTENT_DISPOSITION, value);
    }
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    if let Some(range_value) = headers.get(RANGE).and_then(|v| v.to_str().ok()) {
        let ranges = match parse_range(range_value, file_size) {
            Ok(value) => value,
            Err(_) => {
                return (StatusCode::RANGE_NOT_SATISFIABLE, "invalid range").into_response();
            }
        };
        if let Some(first) = ranges.first() {
            let mut file = match fs::File::open(&file_path) {
                Ok(value) => value,
                Err(error) => return map_file_error(error),
            };
            if let Err(error) = file.seek(SeekFrom::Start(first.start)) {
                return map_file_error(error);
            }
            let mut buffer = vec![0u8; first.length as usize];
            if let Err(error) = file.read_exact(&mut buffer) {
                return map_file_error(error);
            }

            if let Ok(value) = HeaderValue::from_str(&first.length.to_string()) {
                response_headers.insert(CONTENT_LENGTH, value);
            }
            if let Ok(value) = HeaderValue::from_str(&format!(
                "bytes {}-{}/{}",
                first.start,
                first.start + first.length - 1,
                file_size
            )) {
                response_headers.insert(CONTENT_RANGE, value);
            }
            return (StatusCode::PARTIAL_CONTENT, response_headers, buffer).into_response();
        }
    }

    let content = match fs::read(&file_path) {
        Ok(value) => value,
        Err(error) => return map_file_error(error),
    };
    if let Ok(value) = HeaderValue::from_str(&content.len().to_string()) {
        response_headers.insert(CONTENT_LENGTH, value);
    }
    (StatusCode::OK, response_headers, content).into_response()
}

#[derive(Debug, Clone, Copy)]
struct HttpRange {
    start: u64,
    length: u64,
}

fn parse_range(header: &str, size: u64) -> Result<Vec<HttpRange>, String> {
    if !header.starts_with("bytes=") {
        return Err("invalid range".to_string());
    }

    let mut ranges = Vec::new();
    for raw in header["bytes=".len()..].split(',') {
        let range = raw.trim();
        if range.is_empty() {
            continue;
        }
        let Some((start_raw, end_raw)) = range.split_once('-') else {
            return Err("invalid range".to_string());
        };
        let start_raw = start_raw.trim();
        let end_raw = end_raw.trim();

        let mut current = if start_raw.is_empty() {
            let mut suffix = end_raw
                .parse::<u64>()
                .map_err(|_| "invalid range".to_string())?;
            if suffix > size {
                suffix = size;
            }
            HttpRange {
                start: size.saturating_sub(suffix),
                length: suffix,
            }
        } else {
            let start = start_raw
                .parse::<u64>()
                .map_err(|_| "invalid range".to_string())?;
            let length = if end_raw.is_empty() {
                size.saturating_sub(start)
            } else {
                let end = end_raw
                    .parse::<u64>()
                    .map_err(|_| "invalid range".to_string())?;
                if end < start {
                    return Err("invalid range".to_string());
                }
                end - start + 1
            };
            HttpRange { start, length }
        };

        if current.start >= size {
            continue;
        }
        if current.start + current.length > size {
            current.length = size - current.start;
        }
        ranges.push(current);
    }
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_handles_common_forms() {
        let range = parse_range("bytes=0-9", 100).expect("range must parse");
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].start, 0);
        assert_eq!(range[0].length, 10);

        let range = parse_range("bytes=-20", 100).expect("suffix must parse");
        assert_eq!(range[0].start, 80);
        assert_eq!(range[0].length, 20);
    }
}
