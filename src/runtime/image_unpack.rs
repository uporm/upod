use flate2::read::GzDecoder;
use oci_client::manifest::{
    IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE, IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE, IMAGE_LAYER_GZIP_MEDIA_TYPE,
    IMAGE_LAYER_MEDIA_TYPE, IMAGE_LAYER_NONDISTRIBUTABLE_GZIP_MEDIA_TYPE, IMAGE_LAYER_NONDISTRIBUTABLE_MEDIA_TYPE,
};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tar::Archive;

use crate::runtime::image_pull::{ImageError, resolve_image_name};

const DOWNLOAD_ROOT: &str = "images";
const CONTAINER_ROOT: &str = "containers";
const LAYERS_DIR: &str = "layers";
const LAYERS_META_FILE: &str = "layers.meta";
const ROOTFS_DIR: &str = "rootfs";
const OCI_CONFIG_FILE: &str = "config.json";

pub fn unpack(url: &str) -> Result<(), ImageError> {
    let image_name = resolve_image_name(url)?;
    let download_dir = download_root().join(&image_name);
    let target_dir = container_root().join(&image_name);
    let rootfs_dir = target_dir.join(ROOTFS_DIR);
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }
    fs::create_dir_all(&rootfs_dir)?;
    fs::write(target_dir.join(OCI_CONFIG_FILE), b"{}\n")?;
    unpack_from_download(&download_dir, &rootfs_dir)?;
    Ok(())
}

fn download_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DOWNLOAD_ROOT)
}

fn container_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(CONTAINER_ROOT)
}

fn unpack_from_download(download_dir: &Path, target_dir: &Path) -> Result<(), ImageError> {
    let metadata = fs::read_to_string(download_dir.join(LAYERS_META_FILE))?;
    let layers_dir = download_dir.join(LAYERS_DIR);
    for line in metadata.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (index_text, media_type) = line
            .split_once('\t')
            .ok_or_else(|| ImageError::InvalidImageMetadata(line.to_string()))?;
        let index = index_text
            .parse::<usize>()
            .map_err(|_| ImageError::InvalidImageMetadata(line.to_string()))?;
        let layer_file = layers_dir.join(format!("{index:04}.layer"));
        let data = fs::read(layer_file)?;
        apply_layer(target_dir, media_type, data.as_slice())?;
    }
    Ok(())
}

fn apply_layer(root: &Path, media_type: &str, data: &[u8]) -> Result<(), ImageError> {
    match media_type {
        IMAGE_LAYER_MEDIA_TYPE
        | IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE
        | IMAGE_LAYER_NONDISTRIBUTABLE_MEDIA_TYPE => unpack_tar(root, data),
        IMAGE_LAYER_GZIP_MEDIA_TYPE
        | IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE
        | IMAGE_LAYER_NONDISTRIBUTABLE_GZIP_MEDIA_TYPE => unpack_tar_gzip(root, data),
        other => Err(ImageError::UnsupportedLayerMediaType(other.to_string())),
    }
}

fn unpack_tar(root: &Path, data: &[u8]) -> Result<(), ImageError> {
    let cursor = Cursor::new(data);
    let mut archive = Archive::new(cursor);
    let entries = archive.entries()?;
    for entry in entries {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if handle_whiteout(root, &path)? {
            continue;
        }
        entry.unpack_in(root)?;
    }
    Ok(())
}

fn unpack_tar_gzip(root: &Path, data: &[u8]) -> Result<(), ImageError> {
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(decoder);
    let entries = archive.entries()?;
    for entry in entries {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if handle_whiteout(root, &path)? {
            continue;
        }
        entry.unpack_in(root)?;
    }
    Ok(())
}

fn handle_whiteout(root: &Path, path: &Path) -> Result<bool, ImageError> {
    let file_name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => return Ok(false),
    };

    if file_name == ".wh..wh..opq" {
        if let Some(parent) = path.parent() {
            let target_dir = root.join(parent);
            remove_dir_contents(&target_dir)?;
        }
        return Ok(true);
    }

    if let Some(stripped) = file_name.strip_prefix(".wh.") {
        if let Some(parent) = path.parent() {
            let target_path = root.join(parent).join(stripped);
            remove_path(&target_path)?;
        }
        return Ok(true);
    }

    Ok(false)
}

fn remove_dir_contents(dir: &Path) -> Result<(), ImageError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        remove_path(&entry.path())?;
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), ImageError> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::unpack;
    use crate::runtime::image_pull::resolve_image_name;
    use oci_client::manifest::IMAGE_LAYER_MEDIA_TYPE;
    use std::fs;
    use std::io::Cursor;

    #[test]
    fn unpack_prepares_bundle_layout() {
        let image = "docker.1ms.run/nginx:latest";
        let image_name = resolve_image_name(image).unwrap();
        let cwd = std::env::current_dir().unwrap();
        let download_dir = cwd.join("images").join(&image_name);
        let container_dir = cwd.join("containers").join(&image_name);

        if download_dir.exists() {
            let _ = fs::remove_dir_all(&download_dir);
        }
        if container_dir.exists() {
            let _ = fs::remove_dir_all(&container_dir);
        }

        fs::create_dir_all(download_dir.join("layers")).unwrap();
        let layer_data = build_tar_with_single_file("etc/test.conf", b"hello unpack");
        fs::write(download_dir.join("layers").join("0000.layer"), layer_data).unwrap();
        fs::write(
            download_dir.join("layers.meta"),
            format!("0\t{IMAGE_LAYER_MEDIA_TYPE}\n"),
        )
        .unwrap();

        unpack(image).unwrap();

        assert!(container_dir.join("config.json").exists());
        let extracted_file = container_dir.join("rootfs").join("etc/test.conf");
        assert!(extracted_file.exists());
        assert_eq!(fs::read_to_string(extracted_file).unwrap(), "hello unpack");

        let _ = fs::remove_dir_all(&download_dir);
        let _ = fs::remove_dir_all(&container_dir);
    }

    fn build_tar_with_single_file(path: &str, content: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut data);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, path, Cursor::new(content))
                .unwrap();
            builder.finish().unwrap();
        }
        data
    }
}
