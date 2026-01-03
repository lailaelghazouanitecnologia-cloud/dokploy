use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::CloudflareConfig;
use crate::error::{AppError, Result};

const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const TIMEOUT_SECONDS: u64 = 30;

pub struct CloudflareProvider {
    client:  Client,
    token:   String,
    zone_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub id:       String,
    pub name:     String,
    pub type_:    String,
    pub content:  String,
    pub ttl:      u32,
    pub proxied:  bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateDnsRecord {
    #[serde(rename = "type")]
    pub type_:   String,
    pub name:    String,
    pub content: String,
    pub ttl:     u32,
    pub proxied: bool,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    result:  Option<T>,
    errors:  Vec<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code:    u32,
    message: String,
}

#[derive(Debug, Deserialize)]
struct ListResult<T> {
    result: Vec<T>,
}

impl CloudflareProvider {
    pub fn new(config: &CloudflareConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .map_err(|e| AppError::Cloudflare(format!("failed to create client: {e}")))?;

        Ok(Self {
            client,
            token:   config.api_token.clone(),
            zone_id: config.zone_id.clone(),
        })
    }

    pub async fn list_dns_records(&self, name_filter: Option<&str>) -> Result<Vec<DnsRecord>> {
        let mut url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records",
            self.zone_id
        );

        if let Some(name) = name_filter {
            url.push_str(&format!("?name={name}"));
        }

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| AppError::Cloudflare(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Cloudflare(format!("api error {status}: {body}")));
        }

        #[derive(Deserialize)]
        struct DnsRecordRaw {
            id:      String,
            name:    String,
            #[serde(rename = "type")]
            type_:   String,
            content: String,
            ttl:     u32,
            proxied: bool,
        }

        let list: ListResult<DnsRecordRaw> = response
            .json()
            .await
            .map_err(|e| AppError::Cloudflare(format!("parse failed: {e}")))?;

        let records = list.result
            .into_iter()
            .map(|r| DnsRecord {
                id:      r.id,
                name:    r.name,
                type_:   r.type_,
                content: r.content,
                ttl:     r.ttl,
                proxied: r.proxied,
            })
            .collect();

        Ok(records)
    }

    pub async fn create_dns_record(&self, record: CreateDnsRecord) -> Result<DnsRecord> {
        if record.name.is_empty() {
            return Err(AppError::Validation("name cannot be empty".into()));
        }

        if record.content.is_empty() {
            return Err(AppError::Validation("content cannot be empty".into()));
        }

        let valid_types = ["A", "AAAA", "CNAME", "TXT", "MX", "NS"];
        if !valid_types.contains(&record.type_.as_str()) {
            return Err(AppError::Validation(format!(
                "invalid record type: {}",
                record.type_
            )));
        }

        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records",
            self.zone_id
        );

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&record)
            .send()
            .await
            .map_err(|e| AppError::Cloudflare(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Cloudflare(format!("api error {status}: {body}")));
        }

        #[derive(Deserialize)]
        struct DnsRecordRaw {
            id:      String,
            name:    String,
            #[serde(rename = "type")]
            type_:   String,
            content: String,
            ttl:     u32,
            proxied: bool,
        }

        let resp: ApiResponse<DnsRecordRaw> = response
            .json()
            .await
            .map_err(|e| AppError::Cloudflare(format!("parse failed: {e}")))?;

        if !resp.success {
            let msg = resp.errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "unknown error".into());
            return Err(AppError::Cloudflare(msg));
        }

        let raw = resp.result.ok_or_else(|| {
            AppError::Cloudflare("no result in response".into())
        })?;

        Ok(DnsRecord {
            id:      raw.id,
            name:    raw.name,
            type_:   raw.type_,
            content: raw.content,
            ttl:     raw.ttl,
            proxied: raw.proxied,
        })
    }

    pub async fn update_dns_record(
        &self,
        record_id: &str,
        record: CreateDnsRecord,
    ) -> Result<DnsRecord> {
        if record_id.is_empty() {
            return Err(AppError::Validation("record_id cannot be empty".into()));
        }

        if record.name.is_empty() {
            return Err(AppError::Validation("name cannot be empty".into()));
        }

        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records/{record_id}",
            self.zone_id
        );

        let response = self.client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&record)
            .send()
            .await
            .map_err(|e| AppError::Cloudflare(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Cloudflare(format!("api error {status}: {body}")));
        }

        #[derive(Deserialize)]
        struct DnsRecordRaw {
            id:      String,
            name:    String,
            #[serde(rename = "type")]
            type_:   String,
            content: String,
            ttl:     u32,
            proxied: bool,
        }

        let resp: ApiResponse<DnsRecordRaw> = response
            .json()
            .await
            .map_err(|e| AppError::Cloudflare(format!("parse failed: {e}")))?;

        if !resp.success {
            let msg = resp.errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "unknown error".into());
            return Err(AppError::Cloudflare(msg));
        }

        let raw = resp.result.ok_or_else(|| {
            AppError::Cloudflare("no result in response".into())
        })?;

        Ok(DnsRecord {
            id:      raw.id,
            name:    raw.name,
            type_:   raw.type_,
            content: raw.content,
            ttl:     raw.ttl,
            proxied: raw.proxied,
        })
    }

    pub async fn delete_dns_record(&self, record_id: &str) -> Result<()> {
        if record_id.is_empty() {
            return Err(AppError::Validation("record_id cannot be empty".into()));
        }

        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records/{record_id}",
            self.zone_id
        );

        let response = self.client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| AppError::Cloudflare(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Cloudflare(format!("api error {status}: {body}")));
        }

        Ok(())
    }

    pub async fn purge_cache(&self, files: Option<Vec<String>>) -> Result<()> {
        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/purge_cache",
            self.zone_id
        );

        #[derive(Serialize)]
        struct PurgeRequest {
            #[serde(skip_serializing_if = "Option::is_none")]
            files: Option<Vec<String>>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            purge_everything: bool,
        }

        let body = PurgeRequest {
            files: files.clone(),
            purge_everything: files.is_none(),
        };

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Cloudflare(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Cloudflare(format!("purge failed {status}: {body}")));
        }

        Ok(())
    }
}
