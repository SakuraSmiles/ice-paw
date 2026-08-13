//! MCP 传输层抽象 + 远程传输实现（streamable HTTP）
//!
//! Phase 3: 支持远程 MCP Server（GLM Coding Plan 等提供 streamable HTTP 端点）。
//!
//! 设计：
//! - `McpTransport` trait —— stdio（`ExternalMcpServer`）/ http / sse 三种传输的统一抽象。
//!   `McpServerManager` 与 `ExternalToolProxy` 通过 `Arc<dyn McpTransport>` 持有，
//!   **不感知具体传输类型**——这是支持远程 MCP 的核心解耦点。
//! - `HttpMcpTransport` —— streamable HTTP：每请求一次 POST JSON-RPC，
//!   响应可能是单 JSON 或 `text/event-stream`（SSE 封包），由 `parse_response`
//!   按 Content-Type 分流解析。
//! - SSE 传输（`SseMcpTransport`）见后续阶段；GLM 的 3 个 Remote 服务
//!   均支持 streamable HTTP，http 传输即可覆盖。

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

use super::types::{
    JsonRpcRequest, JsonRpcResponse, McpCallToolParams, McpCallToolResult,
    McpClientCapabilities, McpClientInfo, McpInitializeParams, McpListToolsResult,
    McpToolDefinition,
};

/// streamable HTTP 传输的 MCP 协议版本。
///
/// 注：stdio（`ExternalMcpServer`）仍用 `"0.1.0"`（现有 bundled server 兼容，不动）；
/// 远程 streamable HTTP 用新版本。两者各自管自己的版本常量，互不影响。
const HTTP_PROTOCOL_VERSION: &str = "2025-06-18";

// =========================================================================
// McpTransport trait
// =========================================================================

/// MCP 传输层统一接口：连接一个 server 后能列出工具、调用工具、关闭。
///
/// `ExternalMcpServer`（stdio）与 `HttpMcpTransport`/`SseMcpTransport`（远程）
/// 都实现此 trait。生命周期由 `McpServerManager::stop_server` 显式调 `shutdown().await`
/// 管理；stdio 的 `Drop`（notify + kill_on_drop）作 best-effort 兜底，http/sse 无子进程。
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// 获取工具列表（`tools/list`）
    async fn list_tools(&self) -> AppResult<Vec<McpToolDefinition>>;
    /// 执行工具（`tools/call`），返回纯文本结果
    async fn call_tool(&self, name: &str, args: &Value) -> AppResult<String>;
    /// 关闭传输（stdio 关进程 / http 取消后台任务）
    async fn shutdown(&self);
}

/// 把 `McpCallToolResult` 的 `content[].text` 抽出来 join 成纯文本。
///
/// stdio（`ExternalMcpServer::call_tool`）与 HTTP/SSE 传输共用此逻辑，
/// 保证跨传输的「工具结果 → 纯文本」语义一致。
pub(crate) fn extract_text_from_call_result(result: &McpCallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|item| item.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
}

// =========================================================================
// HttpMcpTransport —— streamable HTTP
// =========================================================================

/// 远程 MCP Server 连接器（streamable HTTP）。
///
/// GLM Coding Plan 的 `web_search_prime` / `web_reader` / `zread` 即此模式：
/// `POST {url}` + `Authorization: Bearer <key>` header。
///
/// 设计：每请求一次独立 POST（GLM 是无状态调用），不维持长连接、不用 pending map。
/// `Mcp-Session-Id` 在 initialize 响应头里拿到后缓存，后续请求带上（合规）。
pub struct HttpMcpTransport {
    name: String,
    url: String,
    /// 基础请求头（含 Authorization）；ACCEPT/CONTENT_TYPE 在 new 时注入
    headers: HeaderMap,
    client: reqwest::Client,
    /// MCP 会话 ID（响应头 `Mcp-Session-Id`）；初始化后只读
    session_id: Mutex<Option<String>>,
}

impl HttpMcpTransport {
    /// 建立连接：构造 reqwest client + initialize 握手。
    pub async fn new(name: String, url: String, headers_value: &Value) -> AppResult<Self> {
        let mut headers = build_header_map(headers_value);
        // streamable HTTP：声明可接受单 JSON 或 SSE 流两种响应
        headers.insert(ACCEPT, HeaderValue::from_static("application/json, text/event-stream"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Internal(format!("构建 HTTP client 失败: {e}")))?;

        let transport = Self {
            name,
            url,
            headers,
            client,
            session_id: Mutex::new(None),
        };

        // initialize 握手（post_jsonrpc 会自动缓存响应里的 Mcp-Session-Id）
        let init_params = McpInitializeParams {
            protocol_version: HTTP_PROTOCOL_VERSION.into(),
            capabilities: McpClientCapabilities {},
            client_info: McpClientInfo {
                name: "ice-paw".into(),
                version: "0.2.7".into(),
            },
        };
        let init_value = serde_json::to_value(&init_params)
            .expect("McpInitializeParams 序列化不应失败");
        let resp = transport.post_jsonrpc("initialize", Some(init_value)).await?;
        if resp.result.is_none() {
            // 部分 server（如 GLM）的 error 对象只回 code 不回 message，
            // 这里兜底带上 code，避免出现空的「握手失败: 」。
            let msg = resp.error.as_ref().map(|e| {
                if e.message.is_empty() {
                    format!("error code {}", e.code)
                } else {
                    e.message.clone()
                }
            }).unwrap_or_else(|| "空响应".into());
            return Err(AppError::Internal(format!(
                "MCP HTTP '{}' 握手失败: {}", transport.name, msg
            )));
        }

        // MCP 规范：initialize 后必须发 notifications/initialized 通知，合规的远程 server
        // （如 GLM）才会放行后续 tools/list。Claude Code 官方客户端会发，故能连上。
        transport.post_notification("notifications/initialized").await;

        tracing::info!(
            target: "ice_paw.mcp",
            "远程 MCP Server '{}' 初始化完成 (streamable HTTP)", transport.name
        );
        Ok(transport)
    }

    /// 发一次 JSON-RPC POST 并解析响应。响应头里的 Mcp-Session-Id 会被缓存。
    async fn post_jsonrpc(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> AppResult<JsonRpcResponse> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Uuid::new_v4().to_string(),
            method: method.into(),
            params,
        };

        let mut builder = self.client.post(&self.url).headers(self.headers.clone());
        // 带上 session id（若有，initialize 时尚为 None）
        let sid = self.session_id.lock().await.clone();
        if let Some(sid) = sid {
            builder = builder.header("mcp-session-id", sid);
        }

        let resp = builder
            .json(&req)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!(
                "MCP HTTP '{}' 请求失败 ({method}): {e}", self.name
            )))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "MCP HTTP '{}' {} 返回 {}: {}", self.name, method, status, txt
            )));
        }

        // 缓存响应头里的会话 ID（initialize 首次带来，后续请求带上）
        if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session_id.lock().await = Some(sid.to_string());
        }

        parse_response(resp, &self.name, method).await
    }

    /// 发送 MCP notification（无 id，不期望 JSON-RPC 响应体）。
    ///
    /// 用于 initialize 握手后的 `notifications/initialized`——MCP streamable HTTP 规范
    /// 要求，合规的远程 server（如 GLM）据此放行后续 tools/list。响应通常是 202/200 无 body；
    /// 任何失败只记 warn 不阻断（真正的协议失败会在后续 tools/list 明确报错）。
    async fn post_notification(&self, method: &str) {
        let req = serde_json::json!({ "jsonrpc": "2.0", "method": method });
        let mut builder = self.client.post(&self.url).headers(self.headers.clone());
        let sid = self.session_id.lock().await.clone();
        if let Some(sid) = sid {
            builder = builder.header("mcp-session-id", sid);
        }
        match builder.json(&req).send().await {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                let status = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    target: "ice_paw.mcp",
                    "MCP HTTP '{}' notification({method}) 返回 {status}（已忽略）: {txt}",
                    self.name
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.mcp",
                    "MCP HTTP '{}' notification({method}) 发送失败（已忽略）: {e}",
                    self.name
                );
            }
        }
    }
}

#[async_trait]
impl McpTransport for HttpMcpTransport {
    async fn list_tools(&self) -> AppResult<Vec<McpToolDefinition>> {
        let resp = self.post_jsonrpc("tools/list", None).await?;
        let result = resp.result.ok_or_else(|| {
            let msg = resp.error.map(|e| e.message).unwrap_or_default();
            AppError::Internal(format!("MCP HTTP '{}' tools/list 失败: {}", self.name, msg))
        })?;
        let list: McpListToolsResult = serde_json::from_value(result)
            .map_err(|e| AppError::Internal(format!("解析 MCP tools/list 结果失败: {e}")))?;
        Ok(list.tools)
    }

    async fn call_tool(&self, tool_name: &str, args: &Value) -> AppResult<String> {
        let params = McpCallToolParams {
            name: tool_name.to_string(),
            arguments: Some(args.clone()),
        };
        let params_value = serde_json::to_value(&params)
            .expect("McpCallToolParams 序列化不应失败");
        let resp = self.post_jsonrpc("tools/call", Some(params_value)).await?;

        if let Some(err) = resp.error {
            return Err(AppError::Internal(format!(
                "MCP HTTP '{}' 工具 '{}' 调用失败: {}", self.name, tool_name, err.message
            )));
        }
        let result_value = resp.result.ok_or_else(|| AppError::Internal(format!(
            "MCP HTTP '{}' 工具 '{}' 返回空结果", self.name, tool_name
        )))?;
        let call_result: McpCallToolResult = serde_json::from_value(result_value)
            .map_err(|e| AppError::Internal(format!("解析 MCP 工具结果失败: {e}")))?;
        Ok(extract_text_from_call_result(&call_result))
    }

    async fn shutdown(&self) {
        // streamable HTTP 无长连接/子进程，无需主动关闭。
        // 日志只打 name，绝不打 headers（含 Authorization）。
        tracing::debug!(target: "ice_paw.mcp", "MCP HTTP '{}' shutdown（无操作）", self.name);
    }
}

// =========================================================================
// 辅助函数
// =========================================================================

/// 从 JSON 对象（如 `{"Authorization": "Bearer xxx"}`）构建 HeaderMap。
/// 非法 header 名/值（含非 ASCII 等）跳过并告警，不阻断启动。
fn build_header_map(headers_value: &Value) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Some(obj) = headers_value.as_object() {
        for (k, v) in obj {
            let Some(s) = v.as_str() else {
                tracing::warn!(target: "ice_paw.mcp", "MCP header '{k}' 的值不是字符串，已忽略");
                continue;
            };
            match (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(s)) {
                (Ok(name), Ok(val)) => {
                    map.insert(name, val);
                }
                _ => {
                    tracing::warn!(target: "ice_paw.mcp", "MCP header 非法，已忽略: {k}");
                }
            }
        }
    }
    map
}

/// 按 Content-Type 解析 streamable HTTP 响应。
///
/// - `text/event-stream`：按 SSE 规范分帧（空行分隔 event），每个 event 取 `event:` 类型
///   与 `data:` 载荷（多行 data 用 `\n` 连接）。一个 POST 响应可能含 ping / endpoint /
///   message 等多个 event，必须按帧取 message 的 data，不能无脑拼接（否则拼出非法 JSON）。
/// - 其他（`application/json`）：直接反序列化。
///
/// 解析失败时把 raw body 打进日志便于排错——body 是 MCP 响应内容，不含 Authorization
/// 等敏感头（它们在 header 里），可安全入日志。
async fn parse_response(
    resp: reqwest::Response,
    name: &str,
    method: &str,
) -> AppResult<JsonRpcResponse> {
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let is_sse = ct.contains("text/event-stream");

    // 两种分支都要用 body，先整块读出（失败时也用来打 raw 排错）。
    let bytes = resp.bytes().await.map_err(|e| {
        AppError::Internal(format!("MCP HTTP '{}' {} 读响应 body 失败: {e}", name, method))
    })?;

    let result: Result<JsonRpcResponse, String> = if is_sse {
        parse_sse(&bytes)
    } else {
        serde_json::from_slice::<JsonRpcResponse>(&bytes).map_err(|e| e.to_string())
    };

    result.map_err(|e| {
        let raw = std::str::from_utf8(&bytes).unwrap_or("<非 UTF-8 body>");
        tracing::warn!(
            target: "ice_paw.mcp",
            "MCP HTTP '{}' {} 响应解析失败 [Content-Type={}]: {} | raw body: {:?}",
            name, method, ct, e, raw
        );
        AppError::Internal(format!(
            "MCP HTTP '{}' {} 解析{}响应失败: {e}",
            name, method, if is_sse { "SSE" } else { "JSON" }
        ))
    })
}

/// 解析 SSE 流为 JSON-RPC 响应：按空行分帧，每帧取 `event:` 类型与 `data:` 载荷。
///
/// 优先返回 `message` 事件（或无显式类型——SSE 默认即 message）的 JSON-RPC；
/// 找不到则兜底尝试所有事件的 data。返回 `String` 错误以附上出错的原始片段。
fn parse_sse(bytes: &[u8]) -> Result<JsonRpcResponse, String> {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    // (event 类型, 该 event 的 data 多行用 \n 连接)
    let mut events: Vec<(Option<String>, String)> = Vec::new();
    let mut cur_event: Option<String> = None;
    let mut cur_data: Vec<String> = Vec::new();

    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            // 空行 = event 边界，落盘当前 event
            if !cur_data.is_empty() {
                events.push((cur_event.take(), cur_data.join("\n")));
                cur_data.clear();
            } else {
                cur_event = None;
            }
            continue;
        }
        if line.starts_with(':') {
            continue; // SSE 注释
        }
        if let Some(rest) = line.strip_prefix("event:") {
            cur_event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            // SSE 规范：data 值是 "data:" 之后的内容；若有且仅有一个前导空格则去掉它。
            let payload = rest.strip_prefix(' ').unwrap_or(rest);
            cur_data.push(payload.to_string());
        }
        // id: / retry: 忽略
    }
    // flush 末尾未以空行结尾的 event
    if !cur_data.is_empty() {
        events.push((cur_event.take(), cur_data.join("\n")));
    }

    let mut last_err = String::from("SSE 流中无 data 事件");
    // 第一轮：只看 message / 无类型事件
    for (evt, data) in &events {
        let is_msg = evt.as_deref().is_none_or(|e| e == "message");
        if is_msg {
            match serde_json::from_str::<JsonRpcResponse>(data) {
                Ok(r) => return Ok(r),
                Err(e) => last_err = format!("{data} -> {e}"),
            }
        }
    }
    // 第二轮：兜底，尝试所有事件
    for (_evt, data) in &events {
        match serde_json::from_str::<JsonRpcResponse>(data) {
            Ok(r) => return Ok(r),
            Err(e) => last_err = format!("{data} -> {e}"),
        }
    }
    Err(last_err)
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::types::McpContentItem;
    use serde_json::json;

    #[test]
    fn extract_text_joins_content_items() {
        let result = McpCallToolResult {
            content: vec![
                McpContentItem { kind: "text".into(), text: Some("hello".into()), data: None, mime_type: None },
                // image 项无 text，应被跳过
                McpContentItem { kind: "image".into(), text: None, data: Some("base64...".into()), mime_type: Some("image/png".into()) },
                McpContentItem { kind: "text".into(), text: Some("world".into()), data: None, mime_type: None },
            ],
            is_error: false,
        };
        assert_eq!(extract_text_from_call_result(&result), "hello\nworld");
    }

    #[test]
    fn extract_text_empty_when_no_text() {
        let result = McpCallToolResult {
            content: vec![
                McpContentItem { kind: "image".into(), text: None, data: Some("x".into()), mime_type: None },
            ],
            is_error: false,
        };
        assert_eq!(extract_text_from_call_result(&result), "");
    }

    #[test]
    fn build_header_map_parses_valid() {
        let map = build_header_map(&json!({
            "Authorization": "Bearer xyz",
            "X-Valid": "ok"
        }));
        assert_eq!(map.get("authorization").unwrap(), "Bearer xyz");
        assert_eq!(map.get("x-valid").unwrap(), "ok");
    }

    #[test]
    fn build_header_map_ignores_non_string() {
        let map = build_header_map(&json!({ "Authorization": 123 }));
        assert!(map.get("authorization").is_none());
    }

    #[test]
    fn build_header_map_empty_for_non_object() {
        let map = build_header_map(&json!("not an object"));
        assert!(map.is_empty());
    }
}
