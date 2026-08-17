//! LLM 数据结构 — 发给 LLM 的消息/内容块/流式增量/工具定义

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
///
/// `source_seq`（Phase 2B 阶段 2）：同上语义的事件纪元锚——源自派生行的
/// `first_seq`（消息首现事件 seq）。仅 derive 读路径填充（DB 行读出恒
/// `None`）；MemoryStage 锚点定位 seq 优先、rowid 兜底。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色："system" | "user" | "assistant" | "tool"
    pub role: String,
    /// 消息内容块数组
    pub content: Vec<ContentBlock>,
    /// pipeline 内部追踪：源 MessageRow.rowid（见类型 doc）；`#[serde(skip)]` 不外泄。
    #[serde(skip)]
    pub source_rowid: Option<i64>,
    /// pipeline 内部追踪：源消息首现事件 seq（见类型 doc）；`#[serde(skip)]` 不外泄。
    #[serde(skip)]
    pub source_seq: Option<i64>,
}

impl ChatMessage {
    /// 从纯文本快速构造（等同旧版行为）
    pub fn from_text(role: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: role.into(),
            content: vec![ContentBlock::text(content)],
            source_rowid: None,
            source_seq: None,
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

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
}
