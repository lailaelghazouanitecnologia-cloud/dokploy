use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;

use crate::config::S3Config;
use crate::error::{AppError, Result};

const MAX_OBJECT_SIZE_BYTES: usize = 5 * 1024 * 1024 * 1024;
const KEY_LENGTH_MAX: usize = 1024;

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
            .map_err(|e| AppError::Storage(format!("bucket not accessible: {e}")))?;

        Ok(())
    }

    pub async fn upload(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<ObjectMeta> {
        if key.is_empty() {
            return Err(AppError::Validation("key cannot be empty".into()));
        }

        if key.len() > KEY_LENGTH_MAX {
            return Err(AppError::Validation(format!(
                "key exceeds max length of {KEY_LENGTH_MAX}"
            )));
        }

        if data.len() > MAX_OBJECT_SIZE_BYTES {
            return Err(AppError::Validation(format!(
                "object exceeds max size of {MAX_OBJECT_SIZE_BYTES} bytes"
            )));
        }

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
            .map_err(|e| AppError::Storage(format!("upload failed: {e}")))?;

        Ok(ObjectMeta {
            key:          key.to_string(),
            size_bytes,
            content_type: content_type.map(String::from),
            etag:         response.e_tag,
        })
    }

    pub async fn download(&self, key: &str) -> Result<Vec<u8>> {
        if key.is_empty() {
            return Err(AppError::Validation("key cannot be empty".into()));
        }

        let response = self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("download failed: {e}")))?;

        let data = response
            .body
            .collect()
            .await
            .map_err(|e| AppError::Storage(format!("read body failed: {e}")))?
            .into_bytes()
            .to_vec();

        Ok(data)
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        if key.is_empty() {
            return Err(AppError::Validation("key cannot be empty".into()));
        }

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("delete failed: {e}")))?;

        Ok(())
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        if key.is_empty() {
            return Err(AppError::Validation("key cannot be empty".into()));
        }

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
                Err(AppError::Storage(format!("head object failed: {service_error}")))
            }
        }
    }

    pub async fn list_prefix(&self, prefix: &str, limit: i32) -> Result<Vec<ObjectMeta>> {
        let limit = limit.clamp(1, 1000);

        let response = self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(limit)
            .send()
            .await
            .map_err(|e| AppError::Storage(format!("list failed: {e}")))?;

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
