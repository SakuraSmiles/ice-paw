//! 前端入参 — `send_message` 等 Tauri 命令的输入结构

use super::llm::ContentBlock;
use serde::Deserialize;

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
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
}
