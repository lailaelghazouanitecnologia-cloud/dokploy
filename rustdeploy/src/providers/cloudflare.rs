use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::CloudflareConfig;
use crate::error::{CloudflareError, Result};
use crate::validation::validate_not_empty;

const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";
const TIMEOUT_SECONDS: u64 = 30;

const VALID_RECORD_TYPES: &[&str] = &["A", "AAAA", "CNAME", "TXT", "MX", "NS"];

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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "create_client",
                reason: e.to_string(),
            })?;

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
            url.push_str(&format!("?name={}", urlencoding::encode(name)));
        }

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "list_dns_records",
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CloudflareError::ApiError {
                code: status as u32,
                message: body,
            }.into());
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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "parse_response",
                reason: e.to_string(),
            })?;

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
        validate_not_empty(&record.name, "name")?;
        validate_not_empty(&record.content, "content")?;

        if !VALID_RECORD_TYPES.contains(&record.type_.as_str()) {
            return Err(CloudflareError::InvalidRecordType {
                type_: record.type_.clone(),
            }.into());
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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "create_dns_record",
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CloudflareError::ApiError {
                code: status as u32,
                message: body,
            }.into());
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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "parse_response",
                reason: e.to_string(),
            })?;

        if !resp.success {
            let error = resp.errors.first();
            return Err(CloudflareError::ApiError {
                code: error.map(|e| e.code).unwrap_or(0),
                message: error.map(|e| e.message.clone()).unwrap_or_else(|| "unknown".into()),
            }.into());
        }

        let raw = resp.result.ok_or_else(|| {
            CloudflareError::RequestFailed {
                operation: "create_dns_record",
                reason: "no result in response".into(),
            }
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
        validate_not_empty(record_id, "record_id")?;
        validate_not_empty(&record.name, "name")?;

        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records/{}",
            self.zone_id,
            urlencoding::encode(record_id)
        );

        let response = self.client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&record)
            .send()
            .await
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "update_dns_record",
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CloudflareError::ApiError {
                code: status as u32,
                message: body,
            }.into());
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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "parse_response",
                reason: e.to_string(),
            })?;

        if !resp.success {
            let error = resp.errors.first();
            return Err(CloudflareError::ApiError {
                code: error.map(|e| e.code).unwrap_or(0),
                message: error.map(|e| e.message.clone()).unwrap_or_else(|| "unknown".into()),
            }.into());
        }

        let raw = resp.result.ok_or_else(|| {
            CloudflareError::RequestFailed {
                operation: "update_dns_record",
                reason: "no result in response".into(),
            }
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
        validate_not_empty(record_id, "record_id")?;

        let url = format!(
            "{CLOUDFLARE_API_BASE}/zones/{}/dns_records/{}",
            self.zone_id,
            urlencoding::encode(record_id)
        );

        let response = self.client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "delete_dns_record",
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CloudflareError::ApiError {
                code: status as u32,
                message: body,
            }.into());
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
            .map_err(|e| CloudflareError::RequestFailed {
                operation: "purge_cache",
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CloudflareError::ApiError {
                code: status as u32,
                message: body,
            }.into());
        }

        Ok(())
    }
}
