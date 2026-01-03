use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::GroqConfig;
use crate::error::{GroqError, ValidationError, Result};
use crate::validation::validate_not_empty;

const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";
const TIMEOUT_SECONDS: u64 = 120;
const MAX_TOKENS_DEFAULT: u32 = 4096;
const TEMPERATURE_DEFAULT: f32 = 0.7;
const PROMPT_LENGTH_MAX: usize = 128_000;

const VALID_ROLES: &[&str] = &["system", "user", "assistant"];

pub struct GroqProvider {
    client:   Client,
    api_key:  String,
    model_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub messages:    Vec<ChatMessage>,
    pub max_tokens:  Option<u32>,
    pub temperature: Option<f32>,
    pub stream:      bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role:    String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id:      String,
    pub choices: Vec<ChatChoice>,
    pub usage:   Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatChoice {
    pub index:         u32,
    pub message:       ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens:     u32,
    pub completion_tokens: u32,
    pub total_tokens:      u32,
}

#[derive(Serialize)]
struct ApiRequest {
    model:       String,
    messages:    Vec<ChatMessage>,
    max_tokens:  u32,
    temperature: f32,
    stream:      bool,
}

impl GroqProvider {
    pub fn new(config: &GroqConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .map_err(|e| GroqError::RequestFailed {
                reason: format!("failed to create client: {e}"),
            })?;

        Ok(Self {
            client,
            api_key:  config.api_key.clone(),
            model_id: config.model_id.clone(),
        })
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        if request.messages.is_empty() {
            return Err(ValidationError::Empty { field: "messages" }.into());
        }

        let total_length: usize = request.messages
            .iter()
            .map(|m| m.content.len())
            .sum();

        if total_length > PROMPT_LENGTH_MAX {
            return Err(GroqError::PromptTooLarge {
                length: total_length,
                limit:  PROMPT_LENGTH_MAX,
            }.into());
        }

        for msg in &request.messages {
            if msg.role.is_empty() {
                return Err(ValidationError::Empty { field: "role" }.into());
            }

            if !VALID_ROLES.contains(&msg.role.as_str()) {
                return Err(GroqError::InvalidRole {
                    role: msg.role.clone(),
                }.into());
            }
        }

        let api_request = ApiRequest {
            model:       self.model_id.clone(),
            messages:    request.messages,
            max_tokens:  request.max_tokens.unwrap_or(MAX_TOKENS_DEFAULT),
            temperature: request.temperature.unwrap_or(TEMPERATURE_DEFAULT),
            stream:      false,
        };

        let url = format!("{GROQ_API_BASE}/chat/completions");

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&api_request)
            .send()
            .await
            .map_err(|e| GroqError::RequestFailed {
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GroqError::ApiError {
                status,
                message: body,
            }.into());
        }

        let chat_response = response
            .json::<ChatResponse>()
            .await
            .map_err(|e| GroqError::RequestFailed {
                reason: format!("parse failed: {e}"),
            })?;

        if chat_response.choices.is_empty() {
            return Err(GroqError::EmptyResponse.into());
        }

        Ok(chat_response)
    }

    pub async fn complete(&self, prompt: &str) -> Result<String> {
        validate_not_empty(prompt, "prompt")?;

        let request = ChatRequest {
            messages: vec![ChatMessage {
                role:    "user".to_string(),
                content: prompt.to_string(),
            }],
            max_tokens:  None,
            temperature: None,
            stream:      false,
        };

        let response = self.chat(request).await?;

        let content = response.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(content)
    }

    pub async fn analyze_code(&self, code: &str, language: &str) -> Result<String> {
        validate_not_empty(code, "code")?;

        let system_prompt = format!(
            "You are a code analyzer. Analyze the following {language} code and provide:\n\
            1. A brief summary of what the code does\n\
            2. Any potential issues or bugs\n\
            3. Security concerns if any\n\
            4. Suggestions for improvement\n\
            Be concise and technical."
        );

        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role:    "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role:    "user".to_string(),
                    content: format!("```{language}\n{code}\n```"),
                },
            ],
            max_tokens:  Some(2048),
            temperature: Some(0.3),
            stream:      false,
        };

        let response = self.chat(request).await?;

        let content = response.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(content)
    }

    pub async fn generate_commit_message(&self, diff: &str) -> Result<String> {
        validate_not_empty(diff, "diff")?;

        let system_prompt = "You are a git commit message generator. \
            Generate a concise, conventional commit message for the given diff. \
            Use format: type(scope): description\n\
            Types: feat, fix, docs, style, refactor, test, chore\n\
            Keep it under 72 characters. Only output the commit message, nothing else.";

        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role:    "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role:    "user".to_string(),
                    content: diff.to_string(),
                },
            ],
            max_tokens:  Some(100),
            temperature: Some(0.3),
            stream:      false,
        };

        let response = self.chat(request).await?;

        let content = response.choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default();

        Ok(content)
    }

    pub async fn explain_error(&self, error: &str, context: Option<&str>) -> Result<String> {
        validate_not_empty(error, "error")?;

        let system_prompt = "You are a debugging assistant. \
            Explain the given error message in simple terms and suggest potential fixes. \
            Be concise and actionable.";

        let mut user_content = format!("Error:\n{error}");

        if let Some(ctx) = context {
            user_content.push_str(&format!("\n\nContext:\n{ctx}"));
        }

        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role:    "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role:    "user".to_string(),
                    content: user_content,
                },
            ],
            max_tokens:  Some(1024),
            temperature: Some(0.5),
            stream:      false,
        };

        let response = self.chat(request).await?;

        let content = response.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(content)
    }
}
