use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;

use crate::config::S3Config;
use crate::error::{StorageError, Result};
use crate::validation::validate_s3_key;

const MAX_OBJECT_SIZE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const LIST_LIMIT_MIN: i32 = 1;
const LIST_LIMIT_MAX: i32 = 1000;

pub struct S3Storage {
    client: Client,
    bucket: String,
}

#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub key:          String,
    pub size_bytes:   i64,
    pub content_type: Option<String>,
    pub etag:         Option<String>,
}

impl S3Storage {
    pub async fn new(config: &S3Config) -> Result<Self> {
        let credentials = Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "rustdeploy",
        );

        let mut s3_config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .force_path_style(true);

        if let Some(endpoint) = &config.endpoint {
            s3_config = s3_config.endpoint_url(endpoint);
        }

        let client = Client::from_conf(s3_config.build());

        let storage = Self {
            client,
            bucket: config.bucket.clone(),
        };

        storage.verify_bucket().await?;

        Ok(storage)
    }

    async fn verify_bucket(&self) -> Result<()> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|e| StorageError::BucketNotAccessible {
                bucket: self.bucket.clone(),
            })?;

        Ok(())
    }

    fn validate_key(&self, key: &str) -> Result<()> {
        validate_s3_key(key)?;

        if key.contains('\0') {
            return Err(StorageError::InvalidKey {
                key: key.to_string(),
                reason: "contains null bytes",
            }.into());
        }

        if key.starts_with('/') {
            return Err(StorageError::InvalidKey {
                key: key.to_string(),
                reason: "cannot start with /",
            }.into());
        }

        if key.contains("..") {
            return Err(StorageError::InvalidKey {
                key: key.to_string(),
                reason: "path traversal not allowed",
            }.into());
        }

        Ok(())
    }

    fn validate_size(&self, size: u64) -> Result<()> {
        if size > MAX_OBJECT_SIZE_BYTES {
            return Err(StorageError::SizeLimitExceeded {
                size_bytes: size,
                limit_bytes: MAX_OBJECT_SIZE_BYTES,
            }.into());
        }

        Ok(())
    }

    pub async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<ObjectMeta> {
        self.validate_key(key)?;
        self.validate_size(data.len() as u64)?;

        let size_bytes = data.len() as i64;
        let body = ByteStream::from(data);

        let mut request = self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body);

        if let Some(ct) = content_type {
            request = request.content_type(ct);
        }

        let response = request
            .send()
            .await
            .map_err(|e| StorageError::UploadFailed {
                key: key.to_string(),
                reason: e.to_string(),
            })?;

        Ok(ObjectMeta {
            key:          key.to_string(),
            size_bytes,
            content_type: content_type.map(String::from),
            etag:         response.e_tag,
        })
    }

    pub async fn upload_with_size_check(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: Option<&str>,
        max_size: u64,
    ) -> Result<ObjectMeta> {
        self.validate_key(key)?;

        let size = data.len() as u64;

        if size > max_size {
            return Err(StorageError::SizeLimitExceeded {
                size_bytes: size,
                limit_bytes: max_size,
            }.into());
        }

        self.upload(key, data, content_type).await
    }

    pub async fn download(&self, key: &str) -> Result<Vec<u8>> {
        self.validate_key(key)?;

        let response = self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let service_error = e.into_service_error();
                if service_error.is_no_such_key() {
                    return StorageError::NotFound { key: key.to_string() };
                }
                StorageError::DownloadFailed {
                    key: key.to_string(),
                    reason: service_error.to_string(),
                }
            })?;

        let content_length = response.content_length.unwrap_or(0) as u64;

        if content_length > MAX_OBJECT_SIZE_BYTES {
            return Err(StorageError::SizeLimitExceeded {
                size_bytes: content_length,
                limit_bytes: MAX_OBJECT_SIZE_BYTES,
            }.into());
        }

        let data = response
            .body
            .collect()
            .await
            .map_err(|e| StorageError::DownloadFailed {
                key: key.to_string(),
                reason: e.to_string(),
            })?
            .into_bytes()
            .to_vec();

        Ok(data)
    }

    pub async fn download_with_size_limit(&self, key: &str, max_size: u64) -> Result<Vec<u8>> {
        self.validate_key(key)?;

        let head = self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let service_error = e.into_service_error();
                if service_error.is_not_found() {
                    return StorageError::NotFound { key: key.to_string() };
                }
                StorageError::DownloadFailed {
                    key: key.to_string(),
                    reason: service_error.to_string(),
                }
            })?;

        let size = head.content_length.unwrap_or(0) as u64;

        if size > max_size {
            return Err(StorageError::SizeLimitExceeded {
                size_bytes: size,
                limit_bytes: max_size,
            }.into());
        }

        self.download(key).await
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        self.validate_key(key)?;

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::UploadFailed {
                key: key.to_string(),
                reason: format!("delete failed: {e}"),
            })?;

        Ok(())
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        self.validate_key(key)?;

        let result = self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(e) => {
                let service_error = e.into_service_error();
                if service_error.is_not_found() {
                    return Ok(false);
                }
                Err(StorageError::DownloadFailed {
                    key: key.to_string(),
                    reason: format!("head failed: {service_error}"),
                }.into())
            }
        }
    }

    pub async fn get_metadata(&self, key: &str) -> Result<Option<ObjectMeta>> {
        self.validate_key(key)?;

        let result = self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;

        match result {
            Ok(head) => Ok(Some(ObjectMeta {
                key:          key.to_string(),
                size_bytes:   head.content_length.unwrap_or(0),
                content_type: head.content_type,
                etag:         head.e_tag,
            })),
            Err(e) => {
                let service_error = e.into_service_error();
                if service_error.is_not_found() {
                    return Ok(None);
                }
                Err(StorageError::DownloadFailed {
                    key: key.to_string(),
                    reason: format!("head failed: {service_error}"),
                }.into())
            }
        }
    }

    pub async fn list_prefix(&self, prefix: &str, limit: i32) -> Result<Vec<ObjectMeta>> {
        if !prefix.is_empty() {
            if prefix.contains("..") {
                return Err(StorageError::InvalidKey {
                    key: prefix.to_string(),
                    reason: "path traversal not allowed",
                }.into());
            }
        }

        let limit = limit.clamp(LIST_LIMIT_MIN, LIST_LIMIT_MAX);

        let response = self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(limit)
            .send()
            .await
            .map_err(|e| StorageError::DownloadFailed {
                key: prefix.to_string(),
                reason: format!("list failed: {e}"),
            })?;

        let objects = response.contents.unwrap_or_default();

        let mut result = Vec::with_capacity(objects.len());

        for obj in objects {
            let key = match obj.key {
                Some(k) => k,
                None => continue,
            };

            result.push(ObjectMeta {
                key,
                size_bytes:   obj.size.unwrap_or(0),
                content_type: None,
                etag:         obj.e_tag,
            });
        }

        Ok(result)
    }
}
