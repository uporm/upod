use std::collections::HashMap;

use crate::{
    error::{Result, UpodError},
    models::{
        FileInfo, FileMetadata, Permission, RenameFileItem, ReplaceFileContentItem,
    },
    sandbox::SandboxHandle,
};

impl SandboxHandle {
    /// 批量删除文件
    /// 
    /// DELETE /files?path=...&path=...
    pub async fn remove_files(&self, paths: &[String]) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files", bridge_url);

        let query: Vec<(&str, &String)> = paths.iter().map(|p| ("path", p)).collect();
        let response = self.client.inner.delete(&url).query(&query).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 获取文件信息
    /// 
    /// GET /files/info?path=...&path=...
    pub async fn get_files_info(&self, paths: &[String]) -> Result<HashMap<String, FileInfo>> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/info", bridge_url);

        let query: Vec<(&str, &String)> = paths.iter().map(|p| ("path", p)).collect();
        let response = self.client.inner.get(&url).query(&query).send().await?;
        let status = response.status();
        if status.is_success() {
            let body = response.text().await?;
            match serde_json::from_str::<HashMap<String, FileInfo>>(&body) {
                Ok(data) => Ok(data),
                Err(e) => Err(UpodError::Client(format!("Failed to parse FileInfo map: {} from body: {}", e, body))),
            }
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 批量重命名或移动文件
    /// 
    /// POST /files/mv
    pub async fn rename_files(&self, items: &[RenameFileItem]) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/mv", bridge_url);

        let response = self.client.inner.post(&url).json(items).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 批量设置文件权限
    /// 
    /// POST /files/permissions
    pub async fn chmod_files(&self, permissions: &HashMap<String, Permission>) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/permissions", bridge_url);

        let response = self.client.inner.post(&url).json(permissions).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 按目录与模式搜索文件
    /// 
    /// GET /files/search?path=...&pattern=...
    pub async fn search_files(&self, path: &str, pattern: Option<&str>) -> Result<Vec<FileInfo>> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/search", bridge_url);

        let mut query = vec![("path", path)];
        if let Some(pat) = pattern {
            query.push(("pattern", pat));
        }

        let response = self.client.inner.get(&url).query(&query).send().await?;
        let status = response.status();
        if status.is_success() {
            let body = response.text().await?;
            match serde_json::from_str::<Vec<FileInfo>>(&body) {
                Ok(data) => Ok(data),
                Err(e) => Err(UpodError::Client(format!("Failed to parse FileInfo list: {} from body: {}", e, body))),
            }
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 批量替换文件内容中的目标字符串
    /// 
    /// POST /files/replace
    pub async fn replace_content(&self, items: &HashMap<String, ReplaceFileContentItem>) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/replace", bridge_url);

        let response = self.client.inner.post(&url).json(items).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 上传文件（支持多个文件）
    /// 
    /// POST /files/upload
    pub async fn upload_files(&self, files: &[(FileMetadata, Vec<u8>)]) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/upload", bridge_url);

        let mut form = reqwest::multipart::Form::new();

        for (metadata, content) in files {
            let metadata_json = serde_json::to_string(metadata)
                .map_err(|e| UpodError::Client(format!("Failed to serialize metadata: {}", e)))?;
            
            form = form.part("metadata", reqwest::multipart::Part::text(metadata_json));
            form = form.part("file", reqwest::multipart::Part::bytes(content.clone()));
        }

        let response = self.client.inner.post(&url).multipart(form).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 下载文件
    /// 
    /// GET /files/download?path=...
    pub async fn download_file(&self, path: &str) -> Result<Vec<u8>> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/files/download", bridge_url);

        let response = self.client.inner.get(&url).query(&[("path", path)]).send().await?;
        let status = response.status();
        if status.is_success() {
            let bytes = response.bytes().await?;
            Ok(bytes.to_vec())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 批量创建目录
    /// 
    /// POST /directories
    pub async fn make_directories(&self, directories: &HashMap<String, Permission>) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/directories", bridge_url);

        let response = self.client.inner.post(&url).json(directories).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 批量删除目录
    /// 
    /// DELETE /directories?path=...&path=...
    pub async fn remove_directories(&self, paths: &[String]) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/directories", bridge_url);

        let query: Vec<(&str, &String)> = paths.iter().map(|p| ("path", p)).collect();
        let response = self.client.inner.delete(&url).query(&query).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }
}
