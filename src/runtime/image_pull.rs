use oci_client::client::{ClientConfig, linux_amd64_resolver};
use oci_client::manifest::{
    IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE, IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE, IMAGE_LAYER_GZIP_MEDIA_TYPE,
    IMAGE_LAYER_MEDIA_TYPE, IMAGE_LAYER_NONDISTRIBUTABLE_GZIP_MEDIA_TYPE, IMAGE_LAYER_NONDISTRIBUTABLE_MEDIA_TYPE,
};
use oci_client::secrets::RegistryAuth;
use oci_client::Client;
use oci_client::Reference;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug)]
pub enum ImageError {
    InvalidReference(String),
    InvalidImageName(String),
    InvalidImageMetadata(String),
    Oci(oci_client::errors::OciDistributionError),
    Io(io::Error),
    UnsupportedLayerMediaType(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::InvalidReference(err) => write!(f, "invalid reference: {err}"),
            ImageError::InvalidImageName(err) => write!(f, "invalid image name: {err}"),
            ImageError::InvalidImageMetadata(err) => write!(f, "invalid image metadata: {err}"),
            ImageError::Oci(err) => write!(f, "oci error: {err}"),
            ImageError::Io(err) => write!(f, "io error: {err}"),
            ImageError::UnsupportedLayerMediaType(media_type) => {
                write!(f, "unsupported layer media type: {media_type}")
            }
        }
    }
}

impl std::error::Error for ImageError {}

impl From<oci_client::errors::OciDistributionError> for ImageError {
    fn from(value: oci_client::errors::OciDistributionError) -> Self {
        ImageError::Oci(value)
    }
}

impl From<io::Error> for ImageError {
    fn from(value: io::Error) -> Self {
        ImageError::Io(value)
    }
}

const DOWNLOAD_ROOT: &str = "images";
const LAYERS_DIR: &str = "layers";
const LAYERS_META_FILE: &str = "layers.meta";

pub fn pull(url: &str) -> Result<(), ImageError> {
    let reference = parse_reference(url)?;
    let image_name = resolve_image_name(url)?;
    let download_dir = download_root().join(&image_name);
    fs::create_dir_all(&download_dir)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(pull_async(&reference, &download_dir))?;
    Ok(())
}

fn parse_reference(url: &str) -> Result<Reference, ImageError> {
    Reference::from_str(url).map_err(|err| ImageError::InvalidReference(err.to_string()))
}

pub(crate) fn resolve_image_name(url: &str) -> Result<String, ImageError> {
    let image_name = url.trim();
    if image_name.is_empty() {
        return Err(ImageError::InvalidImageName("empty image reference".to_string()));
    }

    let mut dir_name = String::with_capacity(image_name.len());
    for ch in image_name.chars() {
        if ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | '-' | '_' | '/' | ':' | '@')
        {
            dir_name.push(ch);
        } else {
            dir_name.push('_');
        }
    }
    if dir_name.is_empty() || dir_name.chars().all(|c| c == '_') {
        return Err(ImageError::InvalidImageName(image_name.to_string()));
    }
    for part in dir_name.split('/') {
        if part == "." || part == ".." {
            return Err(ImageError::InvalidImageName(image_name.to_string()));
        }
    }

    Ok(dir_name)
}

fn download_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DOWNLOAD_ROOT)
}

async fn pull_async(reference: &Reference, dir: &Path) -> Result<(), ImageError> {
    let client_config = ClientConfig {
        platform_resolver: Some(Box::new(linux_amd64_resolver)),
        ..Default::default()
    };
    let client = Client::new(client_config);
    let auth = RegistryAuth::Anonymous;
    let accepted_media_types = vec![
        IMAGE_LAYER_MEDIA_TYPE,
        IMAGE_LAYER_GZIP_MEDIA_TYPE,
        IMAGE_DOCKER_LAYER_TAR_MEDIA_TYPE,
        IMAGE_DOCKER_LAYER_GZIP_MEDIA_TYPE,
        IMAGE_LAYER_NONDISTRIBUTABLE_MEDIA_TYPE,
        IMAGE_LAYER_NONDISTRIBUTABLE_GZIP_MEDIA_TYPE,
    ];
    let image = client.pull(reference, &auth, accepted_media_types).await?;
    store_layers(dir, &image.layers)?;
    Ok(())
}

fn store_layers(dir: &Path, layers: &[oci_client::client::ImageLayer]) -> Result<(), ImageError> {
    let layers_dir = dir.join(LAYERS_DIR);
    if layers_dir.exists() {
        fs::remove_dir_all(&layers_dir)?;
    }
    fs::create_dir_all(&layers_dir)?;
    let mut metadata = String::new();
    for (index, layer) in layers.iter().enumerate() {
        let layer_file = layers_dir.join(format!("{index:04}.layer"));
        fs::write(layer_file, layer.data.as_ref())?;
        metadata.push_str(&format!("{index}\t{}\n", layer.media_type));
    }
    fs::write(dir.join(LAYERS_META_FILE), metadata.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{pull, resolve_image_name};
    use crate::runtime::image_unpack::unpack;

    #[test]
    #[ignore]
    fn pull_nginx_image() {
        let image = "docker.1ms.run/nginx:latest";
        let image_name = resolve_image_name(image).unwrap();
        let download_dir = std::env::current_dir().unwrap().join("images").join(&image_name);
        let container_dir = std::env::current_dir()
            .unwrap()
            .join("containers")
            .join(&image_name);
        if download_dir.exists() {
            let _ = std::fs::remove_dir_all(&download_dir);
        }
        if container_dir.exists() {
            let _ = std::fs::remove_dir_all(&container_dir);
        }

        pull(image).unwrap();
        unpack(image).unwrap();

        assert!(container_dir.join("config.json").exists());
        assert!(container_dir.join("rootfs").join("etc/nginx/nginx.conf").exists());
    }
}
