//! `web_fetch` 工具：抓取 URL 正文（reqwest GET）
//!
//! `Always` 授权（只读网络 GET，无副作用）。返回响应正文文本（超长截断）。
//! v1 返回原始正文（HTML/JSON/纯文本由 LLM 自行解读）。

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::McpClient;
use super::types::AuthorizationLevel;

const MAX_CHARS: usize = 20_000;

pub struct WebFetchTool;

#[derive(Deserialize)]
struct WebFetchArgs {
    url: String,
    #[serde(default = "default_max_chars")]
    max_chars: usize,
}

fn default_max_chars() -> usize {
    MAX_CHARS
}

#[async_trait]
impl McpClient for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL via HTTP GET and return the response body as text. Use for documentation, \
API endpoints, or web pages. Output is truncated if very long."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The URL to fetch." },
                "max_chars": { "type": "integer", "default": 20000 }
            },
            "required": ["url"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: WebFetchArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("web_fetch 参数解析失败: {e}")))?;

        let client = reqwest::Client::builder()
            .user_agent("ice-paw/0.1")
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AppError::Internal(format!("构建 HTTP client 失败: {e}")))?;

        let resp = client
            .get(&parsed.url)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("web_fetch 请求失败: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Internal(format!("web_fetch 读取响应失败: {e}")))?;

        let truncated = text.chars().count() > parsed.max_chars;
        let body = if truncated {
            let mut s: String = text.chars().take(parsed.max_chars).collect();
            s.push_str("\n...[已截断]");
            s
        } else {
            text
        };

        Ok(serde_json::json!({
            "url": parsed.url,
            "status": status,
            "truncated": truncated,
            "content": body,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reject_missing_url() {
        let tool = WebFetchTool;
        let result = tool.execute(r#"{}"#).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("参数解析失败") || err.contains("Validation"));
    }

    #[tokio::test]
    async fn reject_invalid_json() {
        let tool = WebFetchTool;
        let result = tool.execute("not json").await;
        assert!(result.is_err());
    }

    #[test]
    fn validate_parameters_schema_has_url() {
        let tool = WebFetchTool;
        let params = tool.parameters();
        let required = params["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("url")));
    }
}
