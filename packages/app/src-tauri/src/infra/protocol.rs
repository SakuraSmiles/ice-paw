//! 跨层协议类型 — 单一数据源
//!
//! 本模块是 chat / llm / commands 跨层共享的协议类型"单一数据源"。
//!
//! 内容分类：
//!
//! 1. **LLM 数据结构**：`ContentBlock` / `ChatMessage` / `ChatDelta` /
//!    `ToolDef` / `TokenUsage`（`LlmProvider` trait 已迁至 harness/provider）
//! 2. **前端入参**：`SendMessageInput` / `TemplateInput`
//!    - `TemplateInput` 当前未在前端发送链路消费（详见 PipelineContext 预留字段）
//! 3. **事件 Payload**：`ChatStartPayload` / `ChatChunkPayload` / `ChatDonePayload` /
//!    `ChatErrorPayload` / `ChatRetryingPayload` / 工具调用 + 思考 payload
//!
//! 图片校验常量与函数已迁至 `infra::image_validation`，此处保留 re-export
//! 以维持现有导入路径兼容。

use serde::{Deserialize, Serialize};

// =========================================================================
// LLM 数据结构
// =========================================================================

/// 消息内容块 — 替代原来的 `content: String`
///
/// 采用 `#[serde(tag = "type")]` 实现多态 JSON 序列化，
/// 与 OpenAI / Anthropic 的 content block 格式自然对齐。
///
/// `PartialEq`：session-events 对账（harness/reconcile.rs）需要逐块比较
/// legacy 行与事件回放两侧；全字段为 String/usize/Option<bool>，值语义安全。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// 文本块
    Text { text: String },
    /// P2-2: 图片块（Vision 输入）
    ///
    /// - `data`：base64 编码的图片数据（**不含** `data:image/...;base64,` 前缀，
    ///   前缀在 adapter 里拼接；这样前端只传裸 base64，存储/校验更干净）
    /// - `media_type`：MIME 类型，支持 `"image/png" | "image/jpeg" | "image/gif" | "image/webp"`
    ///
    /// 序列化格式（与前端 `types/index.ts` 对齐）：
    /// ```json
    /// { "type": "image", "data": "iVBORw0KG...", "media_type": "image/png" }
    /// ```
    Image { data: String, media_type: String },
    /// 工具调用（LLM 产出）
    ToolUse {
        id: String,
        name: String,
        /// JSON 字符串（arguments / input）
        input: String,
    },
    /// 工具结果（回传给 LLM）
    ToolResult {
        tool_use_id: String,
        /// 结果内容（JSON 字符串或纯文本）
        content: String,
        /// 是否出错
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// 思考过程（Anthropic extended thinking）
    Thinking {
        thinking: String,
        /// 签名（Anthropic 用于验证）
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// 附件元信息块（Phase 3 办公文档附件）
    ///
    /// **纯 UI 展示用**：只记录用户上传了什么附件（文件名 / 类型 / 字节数），
    /// 让用户气泡与历史记录能渲染出"上传了 xxx.docx"的卡片。
    /// **绝不发给 LLM**——provider 适配层（anthropic/openai）会显式跳过它
    /// （与 Thinking 同模式：filter_map 返回 None）。LLM 实际读到的是后端
    /// `materialize_file_blocks` 解析出的 Text 块（提取后的正文）。
    ///
    /// `kind`：小写扩展名（`docx`/`xlsx`/`xls`/`pdf`），前端按它选图标/标签。
    /// `size`：解码后字节数（用于显示 "1.2 MB"）。
    ///
    /// 序列化格式（与前端 `types/index.ts` 对齐）：
    /// ```json
    /// { "type": "attachment", "name": "report.docx", "kind": "docx", "size": 12345 }
    /// ```
    /// 注意：`join_text` 只匹配 Text → Attachment 不污染 content_text / query / 标题。
    Attachment {
        name: String,
        kind: String,
        size: usize,
    },
}

impl ContentBlock {
    /// 从纯文本构造 Text block
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// P2-2: 构造 Image block（裸 base64，无 data URL 前缀）
    pub fn image(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentBlock::Image {
            data: data.into(),
            media_type: media_type.into(),
        }
    }

    /// Phase 3: 构造附件元信息 block（name=文件名，kind=小写扩展名，size=字节数）
    pub fn attachment(name: impl Into<String>, kind: impl Into<String>, size: usize) -> Self {
        ContentBlock::Attachment {
            name: name.into(),
            kind: kind.into(),
            size,
        }
    }

    /// 提取纯文本内容（仅 Text 变体有）
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    /// P2-2: 是否是 Image 块
    pub fn is_image(&self) -> bool {
        matches!(self, ContentBlock::Image { .. })
    }

    /// 把所有 Text block 的文本拼接成一个 String（兼容旧代码）
    pub fn join_text(blocks: &[ContentBlock]) -> String {
        let mut buf = String::new();
        for b in blocks {
            if let ContentBlock::Text { text } = b {
                buf.push_str(text);
            }
        }
        buf
    }
}

// Re-export: 图片校验（从 image_validation 迁出，保留兼容路径）
pub use super::image_validation::{
    is_supported_image_media_type, strip_empty_image_blocks, validate_images, MAX_IMAGE_COUNT,
    MAX_IMAGE_SIZE, SUPPORTED_IMAGE_MEDIA_TYPES,
};

/// 聊天消息（发给 LLM 的上下文中的单条）
///
/// P2-1 升级：`content` 改为 `Vec<ContentBlock>`。
/// 对旧消息（纯文本）使用 `ChatMessage::from_text` 构造。
///
/// `source_rowid`（Phase 2）：pipeline 内部追踪字段，记录本条 ChatMessage
/// 源自哪条 `MessageRow.rowid`。`#[serde(skip)]` 保证它**永不**进入 LLM
/// payload 或任何序化路径——仅 `load_history_with_window` 填充、`MemoryStage`
/// 按「值」定位摘要覆盖切断点（identity-by-value，扛得住 ToolFailureFold
/// 的合并/重排）。合成消息（当前用户、注入摘要等）为 `None`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色："system" | "user" | "assistant" | "tool"
    pub role: String,
    /// 消息内容块数组
    pub content: Vec<ContentBlock>,
    /// pipeline 内部追踪：源 MessageRow.rowid（见类型 doc）；`#[serde(skip)]` 不外泄。
    #[serde(skip)]
    pub source_rowid: Option<i64>,
}

impl ChatMessage {
    /// 从纯文本快速构造（等同旧版行为）
    pub fn from_text(role: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: role.into(),
            content: vec![ContentBlock::text(content)],
            source_rowid: None,
        }
    }

    /// 把所有 content block 拼成纯文本（兼容旧逻辑 / DB 回写）
    pub fn content_text(&self) -> String {
        ContentBlock::join_text(&self.content)
    }
}

/// P2-3: Token 用量信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// P2-3: 缓存命中的 token 数（Anthropic: cache_read_input_tokens, OpenAI: cached_tokens）
    #[serde(default)]
    pub cached_tokens: u32,
}

/// 流式增量 — LLM 返回的每个 chunk
///
/// - `Delta`：文本增量（最常见）
/// - `ToolCallStart`：工具调用开始（id + name 已知）
/// - `ToolCallDelta`：工具调用参数 JSON 片段
/// - `ToolCallEnd`：工具调用参数完毕
/// - `Thinking`：思考过程增量
/// - `Usage`：P2-3 token 用量（OpenAI streaming usage 或 Anthropic message_start）
/// - `Done`：流结束（携带结束原因）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatDelta {
    /// 文本增量
    Delta { content: String },
    /// 工具调用开始
    ToolCallStart { id: String, name: String },
    /// 工具调用参数 JSON 增量
    ToolCallDelta { id: String, delta: String },
    /// 工具调用参数完成
    ToolCallEnd { id: String },
    /// 思考过程增量
    Thinking { content: String },
    /// P2-3: Token 用量
    Usage { usage: TokenUsage },
    /// 流结束
    Done { finish_reason: Option<String> },
}

/// 工具定义（发给 LLM 的 tool schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema（parameters）
    pub parameters: serde_json::Value,
}

// Re-export: LlmProvider trait（从 infra/protocol 迁至 harness/provider，保留兼容路径）
pub use crate::harness::provider::LlmProvider;

// =========================================================================
// 入参结构
// =========================================================================

/// `send_message` 入参中的模板部分（P2-4）
///
/// - `template_id`  选中的模板 ID
/// - `values`       变量值字典
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateInput {
    pub template_id: String,
    #[serde(default)]
    pub values: std::collections::HashMap<String, String>,
}

/// `send_message` 入参
///
/// P2-2 双接口：
/// - `content: Option<String>` — 旧接口，纯文本（保持向后兼容）
/// - `content_blocks: Option<Vec<ContentBlock>>` — 新接口，支持图片等多模态块
///
/// 优先级：`content_blocks` 存在时优先使用；否则 fallback 到 `content`。
/// 两者都不提供 → 校验失败（与旧版「content 不能为空」一致）。
///
/// P0-3: 可选 `model` 覆盖 —— 会话级 model override。
/// - `None` 或缺省 → 使用 Agent 配置的默认 model
/// - `Some(name)` → 本次请求使用 `name`（不修改 Agent 配置，仅本次生效）
#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub conversation_id: String,
    /// 旧接口：纯文本（与 P2-1 之前一致）
    /// P2-2 后改为 `Option<String>`，与 `content_blocks` 二选一
    #[serde(default)]
    pub content: Option<String>,
    /// P2-2: 新接口：富文本块（含 Image 等多模态）
    #[serde(default)]
    pub content_blocks: Option<Vec<ContentBlock>>,
    /// P2-1: 是否启用工具调用
    #[serde(default)]
    pub tools_enabled: bool,
    /// P0-3: 会话级 model 覆盖（None = 使用 Agent 默认 model）
    #[serde(default)]
    pub model: Option<String>,
    /// Phase 3: office/pdf 文件附件（docx/xlsx/xls/pdf）。
    ///
    /// **设计**：文件是**输入模态**而非 content block——LLM 读不了 base64 二进制，
    /// 后端在 [`send_message`] 入口把它们提取成 Text 块追加到 content（见
    /// `materialize_file_blocks`），因此不进 `ContentBlock` 枚举、base64 不落盘。
    #[serde(default)]
    pub files: Option<Vec<AttachedFile>>,
}

/// 聊天文件附件（office/pdf）。
///
/// - `name`：文件名（含扩展名），决定解析格式（docx/xlsx/xls/pdf）。
/// - `data`：base64 编码的文件字节（**不含** `data:...;base64,` 前缀，与 Image 约定一致）。
#[derive(Debug, Deserialize, Clone)]
pub struct AttachedFile {
    pub name: String,
    pub data: String,
}

// =========================================================================
// 事件 Payload 结构
// =========================================================================

/// `chat:start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatStartPayload {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    /// 后端 materialize 后的用户消息 content_blocks（含附件提取出的 Text 块）。
    /// 仅当本次发送含 office/pdf 附件时为 Some——前端乐观用户消息只放了 Attachment
    /// 占位卡片、拿不到提取正文，据此就地 patch，让附件详情弹窗能展示提取原文。
    /// None（纯文本/图片消息）时前端不动用户消息。
    pub user_content_blocks: Option<String>,
}

/// `chat:assistant-start` 事件 payload
///
/// 多轮工具调用场景：每轮工具执行完毕、创建下一轮 assistant 占位消息时 emit。
/// 前端据此「冻结上一条 assistant」（把本轮 streaming 文本/思考/工具调用写入其
/// content_blocks，仅含 tool_use 不含 result）+「按 tool_use_id 组装 user(tool_result)
/// 插入」+「重置 streaming 状态」+「push 新 assistant 占位」。
///
/// 与 `chat:start` 区别：`chat:start` 在整次发送开始时由 chat_cmd 发一次（首条
/// assistant）；`chat:assistant-start` 在每轮工具后发（第 2 条及之后的 assistant）。
#[derive(Clone, Serialize)]
pub struct ChatAssistantStartPayload {
    pub conversation_id: String,
    pub message_id: String,
}

/// `chat:delegation-started` 事件 payload（MA-1 UX：运行中即可达）
///
/// 委派子会话**创建成功即发**（run_agent_turn spawn 前，inline）——此前
/// child_conversation_id 只在完成时的 tool_result 里回传，运行中的委派卡片/
/// 任务入口全都跳不进去。前端据此刷新会话列表（子会话行即刻可见，任务胶囊/
/// 运行中卡片即可跳转）。v1 串行执行保证同父同时至多一个运行中委派。
#[derive(Clone, Serialize)]
pub struct DelegationStartedPayload {
    /// 父会话 id（事件路由用）
    pub conversation_id: String,
    /// 新建的委派子会话 id
    pub child_conversation_id: String,
    /// 专家 agent 显示名
    pub agent_name: String,
    /// 子会话标题（task 截断文本，UX #4 已去「委派: 」前缀）
    pub title: String,
}

/// `chat:chunk` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatChunkPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub delta: String,
}

/// `chat:done` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatDonePayload {
    pub conversation_id: String,
    pub message_id: String,
    pub finish_reason: String,
    /// P2-3: Token 用量信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// `chat:error` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatErrorPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub kind: String,
    pub message: String,
}

/// `chat:round-state` 事件 payload — W2.4 可观测性
#[derive(Clone, Serialize, Debug)]
pub struct ChatRoundStatePayload {
    pub conversation_id: String,
    pub round: u32,
    pub elapsed_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_completion: u32,
    pub cached_tokens: u32,
    pub retry_count: u32,
}

/// `chat:retrying` 事件 payload — 通知前端正在重试
#[derive(Clone, Serialize)]
pub struct ChatRetryingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
    /// W2.6: 重试原因（如 "network_error" / "server_error_5xx"）
    pub reason: String,
}

// === P2-1 工具调用事件 payload ===

/// `chat:tool-call-start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallStartPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub name: String,
}

/// `chat:tool-call-delta` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallDeltaPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub delta: String,
}

/// `chat:tool-call-end` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallEndPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
}

/// `chat:tool-result` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolResultPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
    /// 工具执行耗时（毫秒），含授权等待
    pub duration_ms: u64,
}

/// `chat:thinking` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatThinkingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
}

/// `chat:summary-injected` 事件 payload（M1.5 A3-4 滚动摘要）
///
/// 当 MemoryStage 触发摘要压缩后，通过此事件通知前端。
#[derive(Clone, Serialize)]
pub struct ChatSummaryInjectedPayload {
    pub conversation_id: String,
    pub summary_tokens: u32,
    pub original_count: u32,
    pub kept_count: u32,
}

// === A2-3 工具授权事件 payload ===

/// `chat:tool-auth-request` 事件 payload (Rust → Frontend)
///
/// 当工具调用需要用户确认授权（例如路径不在白名单）时，Rust 侧 emit 此事件，
/// 携带工具名 / 待访问路径 / 参数 / 唯一 request_id，前端弹窗后用同一
/// `request_id` 响应。
///
/// - `request_id`     唯一标识，前后端匹配响应用
/// - `tool_use_id`    LLM 端的工具调用 ID（用于工具结果回填）
/// - `tool_name`      工具名
/// - `file_path`      待访问的路径（可能为空，例如 list_directory 也适用）
/// - `arguments`      工具调用参数 JSON 字符串（前端展示用）
/// - `conversation_id` / `message_id` 与其它 chat:* 事件保持一致，便于前端过滤
/// - `reason`         触发原因（前端展示文案）
#[derive(Clone, Serialize)]
pub struct ToolAuthRequestPayload {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub file_path: String,
    pub arguments: String,
    pub conversation_id: String,
    pub message_id: String,
    pub reason: String,
}

/// 授权范围（#11 分层授权记忆）：用户在审批卡上选择的「允许」生效档位。
/// 默认 `Once`（仅本次）；`ThisDir`/`ThisTool` 记入会话级授权记忆，
/// 本会话内同范围不再询问（流结束即清，不跨会话持久）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    /// 仅本次（等价旧行为：精确路径入会话记忆）
    #[default]
    Once,
    /// 此目录（含子目录）会话内免问；无路径工具退化为工具档
    ThisDir,
    /// 此工具会话内免问（Confirm 级工具唯一可用的扩围档）
    ThisTool,
}

/// `chat:tool-auth-response` 事件 payload (Frontend → Rust)
///
/// 前端弹窗后通过此事件把用户选择告诉 Rust 侧。
/// Rust 侧在 `tool_executor` 里用 `request_id` 匹配 oneshot 通道，
/// 据此决定执行工具还是把工具结果写为「拒绝授权」。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolAuthResponse {
    pub request_id: String,
    pub allowed: bool,
    /// 允许的生效范围（拒绝时忽略）；`#[serde(default)]` 兼容旧前端
    #[serde(default)]
    pub scope: AuthScope,
}

// === 配置提案事件 ===

/// 敏感度分级（贯穿所有阶段的调节阀）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityTier {
    /// 🟢 非敏感：改 agent 名/温度/system_prompt、enable MCP、设 workspace、改时区
    #[serde(rename = "low")]
    Low,
    /// 🟡 敏感：API Key、新建带工具的 agent、创建 MCP server、改 embedding 配置
    #[serde(rename = "medium")]
    Medium,
    /// 🔴 红线：删除、跨 agent 改动、提权、读回密钥明文（提案路径根本不受理）
    #[serde(rename = "redline")]
    Redline,
}

/// 提案动作（Phase 1 仅 agent 域 create/update）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProposalAction {
    /// 创建 agent（🟢 无工具 / 🟡 带 enabled_tools）
    CreateAgent {
        id: String,
        name: String,
        provider: String,
        model: String,
        /// 🔴 绝对不能填真实 key，只能是 "__SLOT__" 占位
        api_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_tools: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
    },
    /// 更新 agent（只能更新当前 agent 自己）
    UpdateAgent {
        agent_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_tools: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
    },
}

/// `chat:config-proposal` 事件 payload（Rust → Frontend）
#[derive(Clone, Serialize)]
pub struct ConfigProposalPayload {
    pub request_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub sensitivity: SensitivityTier,
    pub action: ProposalAction,
    /// 人类可读的提案摘要（agent 生成，前端展示用）
    pub summary: String,
}

/// `chat:config-proposal-response` 事件 payload（Frontend → Rust）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigProposalResponse {
    pub request_id: String,
    #[serde(rename = "decision")]
    pub decision: ProposalDecision,
}

/// 用户对提案的决定
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    /// 用户批准，前端已通过现有可信命令执行
    Approved,
    /// 用户修改后批准
    Modified {
        /// 被修改的字段名 → 新值（JSON string）
        #[serde(default)]
        changes: std::collections::HashMap<String, String>,
    },
    /// 用户拒绝
    Rejected {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// 提案/授权请求失效（超时/通道关闭）时 Rust→Frontend 通知。
///
/// 前端按 `request_id` 清除对应的待处理卡片/弹窗，避免用户对已失效请求
/// 操作（例如对已超时的提案点「批准」会真的创建 agent，但对话流已当它
/// 取消 → 状态分裂、留下孤儿 agent）。
///
/// 用于 `chat:config-proposal-cancel` 与 `chat:tool-auth-request-cancel` 两个事件。
#[derive(Clone, Serialize)]
pub struct PendingRequestCancelPayload {
    pub request_id: String,
    pub conversation_id: String,
    /// "timeout" | "cancelled" | "abort"
    pub reason: String,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use base64::Engine as _;

    /// 构造 N 字节原始数据 → base64 字符串
    fn make_b64_bytes(n: usize) -> String {
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; n])
    }

    // --- validate_images ---

    #[test]
    fn validate_images_empty_blocks_ok() {
        // 无图片 → 直接通过
        assert!(validate_images(&[]).is_ok());
        let blocks = vec![ContentBlock::text("纯文本")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_small_image_ok() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(1024), "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_too_large_rejected() {
        // 6 MiB > 5 MiB 上限
        let big = make_b64_bytes(6 * 1024 * 1024);
        let blocks = vec![ContentBlock::image(big, "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("过大"), "错误信息应提示过大，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_5mb_ok() {
        // 5 MiB 边界值应放行
        let exact = make_b64_bytes(MAX_IMAGE_SIZE);
        let blocks = vec![ContentBlock::image(exact, "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_5mb_plus_one_rejected() {
        let over = make_b64_bytes(MAX_IMAGE_SIZE + 1);
        let blocks = vec![ContentBlock::image(over, "image/png")];
        assert!(validate_images(&blocks).is_err());
    }

    #[test]
    fn validate_images_unsupported_media_type_rejected() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(100), "image/bmp")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(
                    msg.contains("不支持"),
                    "错误信息应提示不支持，实际: {}",
                    msg
                );
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_invalid_base64_rejected() {
        let blocks = vec![ContentBlock::image("not_base64!@#$%", "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(
                    msg.contains("base64"),
                    "错误信息应提到 base64，实际: {}",
                    msg
                );
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_count_limit() {
        // 21 张 1KB 图片 → 超过 MAX_IMAGE_COUNT=20
        let blocks: Vec<ContentBlock> = (0..21)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("最多"), "错误信息应提到最多，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_max_count_ok() {
        // 恰好 20 张 → 应放行
        let blocks: Vec<ContentBlock> = (0..MAX_IMAGE_COUNT)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();

        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_mixed_text_and_images_ok() {
        // 文本 + 多张图片混合
        let mut blocks = vec![ContentBlock::text("看这些图")];
        for _ in 0..3 {
            blocks.push(ContentBlock::image(make_b64_bytes(1024), "image/png"));
        }
        blocks.push(ContentBlock::text("请描述"));
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_supports_all_four_types() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            let blocks = vec![ContentBlock::image(make_b64_bytes(100), mt)];
            assert!(validate_images(&blocks).is_ok(), "{} 应被允许", mt);
        }
    }

    // --- strip_empty_image_blocks（0 字节图片软剥离）---

    #[test]
    fn strip_keeps_nonempty_image() {
        // 非空图片（1KB）→ 原样保留，无提示注入
        let blocks = vec![
            ContentBlock::text("看图"),
            ContentBlock::image(make_b64_bytes(1024), "image/png"),
        ];
        let out = strip_empty_image_blocks(blocks);
        assert_eq!(out.len(), 2, "非空图片应保留，无额外提示");
        assert!(out[1].is_image());
    }

    #[test]
    fn strip_removes_empty_image_and_injects_hint() {
        // 0 字节图片（data 为空）→ 剥离 + 注入诚实提示
        let blocks = vec![
            ContentBlock::image(String::new(), "image/png"),
            ContentBlock::text("正文"),
        ];
        let out = strip_empty_image_blocks(blocks);
        // 期望：空图被移除，正文保留，末尾追加 1 条提示
        assert_eq!(out.len(), 2, "空图剥离后应为 正文 + 提示");
        assert_eq!(out[0].as_text(), Some("正文"));
        let hint = out[1].as_text().expect("末尾应追加提示");
        assert!(hint.contains("0 字节"), "提示应说明 0 字节，实际: {hint}");
        assert!(out.iter().all(|b| !b.is_image()), "不应残留任何图片块");
    }

    #[test]
    fn strip_keeps_valid_when_mixed_with_empty() {
        // 一空一有效：空图剥离、有效图保留、提示点名「第 1 张」
        let blocks = vec![
            ContentBlock::image(String::new(), "image/png"), // 第 1 张：空
            ContentBlock::image(make_b64_bytes(512), "image/jpeg"), // 第 2 张：有效
        ];
        let out = strip_empty_image_blocks(blocks);
        let images: Vec<_> = out.iter().filter(|b| b.is_image()).collect();
        assert_eq!(images.len(), 1, "仅保留 1 张有效图");
        let hint = out.iter().find_map(|b| b.as_text()).expect("应有提示");
        assert!(hint.contains("第 1 张"), "应点名第 1 张为空，实际: {hint}");
    }

    #[test]
    fn strip_no_image_unchanged() {
        // 纯文本 / 无图 → 原样返回、无提示
        let blocks = vec![ContentBlock::text("只有文字")];
        let out = strip_empty_image_blocks(blocks);
        assert_eq!(out.len(), 1);
        assert!(out[0].as_text().is_some());
    }

    #[test]
    fn strip_invalid_base64_treated_as_empty() {
        // 非法 base64（解码失败）→ 视作坏块剥离（发送必失败）
        let blocks = vec![ContentBlock::image("not_base64!@#$%", "image/png")];
        let out = strip_empty_image_blocks(blocks);
        assert!(
            out.iter().all(|b| !b.is_image()),
            "非法 base64 图片应被剥离"
        );
        assert!(out.iter().any(|b| b.as_text().is_some()), "应注入提示");
    }

    // --- SendMessageInput 序列化（确认前端 JSON 格式） ---

    #[test]
    fn send_input_accepts_legacy_content() {
        // 旧版 JSON（仅 content）应能反序列化
        let json = r#"{"conversation_id":"c1","content":"hello"}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert_eq!(input.content.as_deref(), Some("hello"));
        assert!(input.content_blocks.is_none());
        assert!(!input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_content_blocks() {
        // 新版 JSON（含 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content_blocks": [
                {"type": "text", "text": "看图"},
                {"type": "image", "data": "AAAA", "media_type": "image/png"}
            ],
            "tools_enabled": true
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        let blocks = input.content_blocks.unwrap();
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "看图"),
            _ => panic!("第一个应为 Text"),
        }
        match &blocks[1] {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "AAAA");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("第二个应为 Image"),
        }
        assert!(input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_both_legacy_and_new() {
        // 同时传 content 和 content_blocks → 都应能反序列化
        // （后端逻辑会优先使用 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content": "legacy text",
            "content_blocks": [
                {"type": "text", "text": "new text"}
            ]
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.content.as_deref(), Some("legacy text"));
        assert!(input.content_blocks.is_some());
    }

    #[test]
    fn send_input_minimal_required_fields() {
        // 仅 conversation_id + content_blocks → 其它字段默认值正确
        let json = r#"{"conversation_id":"c1","content_blocks":[]}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        // 空数组 → Some(vec![])，后续逻辑会 fallback 到 legacy_content 校验
        let blocks = input.content_blocks.unwrap();
        assert!(blocks.is_empty());
        assert!(!input.tools_enabled);
    }

    // --- ContentBlock / 白名单（从 llm/mod.rs 迁入）---

    #[test]
    fn image_block_serde_roundtrip() {
        let block = ContentBlock::image("iVBORw0KGgo=", "image/png");
        let json = serde_json::to_string(&block).unwrap();
        // tag = "type", rename_all = "snake_case"
        assert_eq!(
            json,
            r#"{"type":"image","data":"iVBORw0KGgo=","media_type":"image/png"}"#
        );
        // 反序列化回原值
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "iVBORw0KGgo=");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("反序列化后类型不对：{:?}", back),
        }
    }

    #[test]
    fn image_block_helper() {
        let b = ContentBlock::image("abc", "image/jpeg");
        assert!(b.is_image());
        assert!(b.as_text().is_none());
    }

    #[test]
    fn text_block_not_image() {
        let b = ContentBlock::text("hello");
        assert!(!b.is_image());
        assert_eq!(b.as_text(), Some("hello"));
    }

    #[test]
    fn supported_media_types_whitelist() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            assert!(is_supported_image_media_type(mt), "{} 应在白名单内", mt);
        }
        for mt in ["image/bmp", "image/svg+xml", "application/pdf", "", "png"] {
            assert!(!is_supported_image_media_type(mt), "{} 不应在白名单内", mt);
        }
    }

    #[test]
    fn join_text_skips_images() {
        // P2-2: join_text 只拼接 Text 块，忽略 Image/ToolUse 等
        let blocks = vec![
            ContentBlock::text("hello "),
            ContentBlock::image("xxxx", "image/png"),
            ContentBlock::text("world"),
        ];
        assert_eq!(ContentBlock::join_text(&blocks), "hello world");
    }

    /// 混合消息（含图片）的 JSON 序列化结构对齐前端 types/index.ts
    #[test]
    fn mixed_message_json_shape() {
        let blocks = vec![
            ContentBlock::text("看这张图"),
            ContentBlock::image("AAAA", "image/png"),
        ];
        let json = serde_json::to_string(&blocks).unwrap();
        // 验证 JSON 数组中两个对象的结构
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "看这张图");

        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["data"], "AAAA");
        assert_eq!(arr[1]["media_type"], "image/png");
        // Image 没有其他字段
        assert_eq!(arr[1].as_object().unwrap().len(), 3);
    }

    // --- A2-3 ToolAuthRequestPayload / ToolAuthResponse ---

    #[test]
    fn tool_auth_request_payload_serde() {
        use super::ToolAuthRequestPayload;
        let p = ToolAuthRequestPayload {
            request_id: "req-1".into(),
            tool_use_id: "tc-1".into(),
            tool_name: "read_file".into(),
            file_path: "/etc/passwd".into(),
            arguments: r#"{"path":"/etc/passwd"}"#.into(),
            conversation_id: "c-1".into(),
            message_id: "m-1".into(),
            reason: "路径 '/etc/passwd' 不在白名单中".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["request_id"], "req-1");
        assert_eq!(parsed["tool_use_id"], "tc-1");
        assert_eq!(parsed["tool_name"], "read_file");
        assert_eq!(parsed["file_path"], "/etc/passwd");
        assert_eq!(parsed["conversation_id"], "c-1");
        assert_eq!(parsed["message_id"], "m-1");
        // 8 个字段
        assert_eq!(parsed.as_object().unwrap().len(), 8);
    }

    #[test]
    fn tool_auth_response_serde_roundtrip() {
        use super::ToolAuthResponse;
        let r = ToolAuthResponse {
            request_id: "req-2".into(),
            allowed: true,
            scope: super::AuthScope::ThisDir,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"request_id":"req-2","allowed":true,"scope":"this_dir"}"#
        );

        let back: ToolAuthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, "req-2");
        assert!(back.allowed);
        assert_eq!(back.scope, super::AuthScope::ThisDir);

        // 旧前端缺 scope 字段 → serde default = Once（向后兼容）
        let legacy: ToolAuthResponse =
            serde_json::from_str(r#"{"request_id":"req-x","allowed":true}"#).unwrap();
        assert_eq!(legacy.scope, super::AuthScope::Once);

        // false 路径
        let r2 = ToolAuthResponse {
            request_id: "req-3".into(),
            allowed: false,
            scope: super::AuthScope::Once,
        };
        let json2 = serde_json::to_string(&r2).unwrap();
        let back2: ToolAuthResponse = serde_json::from_str(&json2).unwrap();
        assert!(!back2.allowed);
    }

    // --- ConfigProposal payload serde ---

    #[test]
    fn sensitivity_tier_serde() {
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Low).unwrap(),
            r#""low""#
        );
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Medium).unwrap(),
            r#""medium""#
        );
        let tier: SensitivityTier = serde_json::from_str(r#""medium""#).unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
    }

    #[test]
    fn proposal_action_create_agent_serde() {
        let action = ProposalAction::CreateAgent {
            id: "test-id".into(),
            name: "Test Agent".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            api_key: "__SLOT__".into(),
            base_url: None,
            system_prompt: Some("You are helpful.".into()),
            temperature: Some(0.7),
            max_tokens: None,
            enabled_tools: None,
            workspace_path: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"create_agent""#));
        assert!(json.contains(r#""api_key":"__SLOT__""#));
        // 反序列化
        let back: ProposalAction = serde_json::from_str(&json).unwrap();
        match back {
            ProposalAction::CreateAgent { id, name, .. } => {
                assert_eq!(id, "test-id");
                assert_eq!(name, "Test Agent");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn proposal_action_update_agent_serde() {
        let action = ProposalAction::UpdateAgent {
            agent_id: "a1".into(),
            name: Some("Renamed".into()),
            provider: None,
            model: None,
            system_prompt: None,
            base_url: None,
            temperature: Some(0.3),
            max_tokens: None,
            enabled_tools: None,
            workspace_path: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"update_agent""#));
        let back: ProposalAction = serde_json::from_str(&json).unwrap();
        match back {
            ProposalAction::UpdateAgent { agent_id, .. } => {
                assert_eq!(agent_id, "a1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn config_proposal_response_serde() {
        let r = ConfigProposalResponse {
            request_id: "req-1".into(),
            decision: ProposalDecision::Approved,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""decision":"approved""#));

        let r2 = ConfigProposalResponse {
            request_id: "req-2".into(),
            decision: ProposalDecision::Rejected {
                reason: Some("不需要".into()),
            },
        };
        let json2 = serde_json::to_string(&r2).unwrap();
        assert!(json2.contains(r#""reason":"不需要""#));
    }
}
