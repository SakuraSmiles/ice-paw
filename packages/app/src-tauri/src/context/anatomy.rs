//! 上下文组成清单（③ 上下文开销可观测化）。
//!
//! 事后纯函数聚合：在 Pipeline 全部 Stage（含 Memory fold / TokenWindow 裁剪 /
//! ModalCapability 剥图——它们都原地改 `ctx.history_messages` / `ctx.final_blocks`）
//! 跑完后调用 [`build_anatomy`]，得到「模型实际看到的 prompt 组成」的段级估算。
//! 不做 Stage 推进式登记的原因：history / summary / user 段必须取裁剪后终值。
//!
//! 产物两路消费：
//! - `context_breakdown` 事件（harness/event_log.rs）→ 轨迹页「上下文组成」区；
//! - 缓存 miss 归因的请求体指纹（`harness/loop/turn_cost.rs`）。
//!
//! ⚠️ 估算口径披露（不变式）：est 是本地估算（CJK 1 token/字、其余约 4 字符/token；
//! 工具 JSON 偏低估），与 provider 实际计数存在偏差——展示侧必须带「估算」标注与
//! 偏差行，措辞诚实（推断非 provider 事实）。

use crate::context::pipeline::PipelineContext;
use crate::context::token::{estimate_block_tokens, estimate_messages_tokens, estimate_tokens};
use crate::infra::protocol::ToolDef;

// =========================================================================
// 段词表（前后端共用字符串；前端中文映射在 TrajectoryInspector）
// =========================================================================

pub(crate) const LABEL_SYSTEM_PERSONA: &str = "system_persona";
pub(crate) const LABEL_SYSTEM_TOOL_HINT: &str = "system_tool_hint";
pub(crate) const LABEL_SYSTEM_OS_CONTEXT: &str = "system_os_context";
pub(crate) const LABEL_SYSTEM_DELEGATION_HINT: &str = "system_delegation_hint";
pub(crate) const LABEL_SYSTEM_CHANNEL_HINT: &str = "system_channel_hint";
pub(crate) const LABEL_SYSTEM_WORD_STYLE: &str = "system_word_style";
pub(crate) const LABEL_TOOL_DEFS: &str = "tool_defs";
pub(crate) const LABEL_SUMMARY: &str = "summary";
pub(crate) const LABEL_HISTORY: &str = "history";
pub(crate) const LABEL_USER_MESSAGE: &str = "user_message";
pub(crate) const LABEL_USER_IMAGES: &str = "user_images";

// =========================================================================
// 结构
// =========================================================================

/// 单段组成：label + 估算 token 数 + 可选计数（工具数/消息数/图片数）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnatomySegment {
    pub label: &'static str,
    pub est: u64,
    pub count: Option<u32>,
}

/// Pipeline 出口的组成清单 + 请求体稳定指纹。
///
/// `system_stable_hash` / `os_stable_hash` 供跨回合 miss 归因比对（工具列表指纹
/// 在 loop 层逐轮计算——Pipeline 不知道每轮的工具子集，见 turn_cost.rs）。
#[derive(Debug, Clone)]
pub(crate) struct ContextAnatomy {
    pub segments: Vec<AnatomySegment>,
    pub system_stable_hash: String,
    pub os_stable_hash: String,
}

// =========================================================================
// 稳定哈希
// =========================================================================

/// FNV-1a 64 位 → 12 hex，字段间混入 `0x1f` 分隔字节防拼接歧义。
///
/// 手写不依赖 std Hasher 的跨版本稳定性——落库数据（context_breakdown 事件的
/// fingerprint 字段）要求重放同值。写法与 agent_profile_migration.rs 的
/// `content_hash` 同源（勿改用 DefaultHasher）。
pub(crate) fn fnv1a_12hex(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // 字段边界混入分隔字节：防相邻字段拼接歧义（[ab,c] ≠ [a,bc]）
        h ^= 0x1f;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:012x}")
}

/// 工具列表指纹：按**发送序**（出口恒按 name 排序，mcp/client.rs 前缀缓存前提）
/// 对每个定义的 name+description+parameters JSON 逐段喂 FNV——顺序与内容都敏感
/// （15+ 工具时相关性选子集逐轮抖动 = miss 真实来源，哈希必须能看见）。
pub(crate) fn hash_tool_defs(defs: &[ToolDef]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |part: &str, h: &mut u64| {
        for b in part.as_bytes() {
            *h ^= u64::from(*b);
            *h = h.wrapping_mul(0x0100_0000_01b3);
        }
        *h ^= 0x1f;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for def in defs {
        mix(&def.name, &mut h);
        mix(&def.description, &mut h);
        mix(&def.parameters.to_string(), &mut h);
    }
    format!("{h:012x}")
}

// =========================================================================
// 聚合
// =========================================================================

fn push_seg(segs: &mut Vec<AnatomySegment>, label: &'static str, est: usize, count: Option<u32>) {
    if est > 0 {
        segs.push(AnatomySegment {
            label,
            est: est as u64,
            count,
        });
    }
}

/// 从 Pipeline 终态聚合组成清单（est=0 段省略）。
///
/// system 五段依赖 `ctx.system_parts`（SystemPromptStage 产物；生产恒 Some，
/// 散落测试构造为 None 时跳过 system 段——诊断视图不猜）。
/// `tool_defs` 段不在此处：工具列表是 loop 层逐轮组装的，由 TurnCostRecorder
/// 并入（begin_round 轮 0 的 est/count）。
pub(crate) fn build_anatomy(ctx: &PipelineContext) -> ContextAnatomy {
    let mut segments: Vec<AnatomySegment> = Vec::new();

    if let Some(parts) = &ctx.system_parts {
        push_seg(&mut segments, LABEL_SYSTEM_PERSONA, parts.persona.as_deref().map_or(0, estimate_tokens), None);
        push_seg(&mut segments, LABEL_SYSTEM_TOOL_HINT, parts.tool_hint.map_or(0, estimate_tokens), None);
        push_seg(&mut segments, LABEL_SYSTEM_OS_CONTEXT, parts.os_context.as_deref().map_or(0, estimate_tokens), None);
        push_seg(
            &mut segments,
            LABEL_SYSTEM_DELEGATION_HINT,
            parts.delegation_hint.as_deref().map_or(0, estimate_tokens),
            None,
        );
        push_seg(
            &mut segments,
            LABEL_SYSTEM_CHANNEL_HINT,
            parts.channel_hint.map_or(0, estimate_tokens),
            None,
        );
        push_seg(
            &mut segments,
            LABEL_SYSTEM_WORD_STYLE,
            parts.word_style.as_deref().map_or(0, estimate_tokens),
            None,
        );
    }

    if let Some(summary) = &ctx.summary {
        push_seg(&mut segments, LABEL_SUMMARY, estimate_tokens(summary), None);
    }

    push_seg(
        &mut segments,
        LABEL_HISTORY,
        estimate_messages_tokens(&ctx.history_messages),
        Some(ctx.history_messages.len() as u32),
    );

    // 当前用户消息：user_prefix（生产恒空，TemplateStage 空转）+ final_blocks 拆
    // 文本/图片两段（ModalCapability 之后的终值——非视觉 agent 的图已代读为 Text）。
    let mut user_text = estimate_tokens(&ctx.rendered_user_prefix);
    let mut image_tokens = 0usize;
    let mut image_count = 0u32;
    for block in &ctx.final_blocks {
        match block {
            crate::infra::protocol::ContentBlock::Image { .. } => {
                image_tokens += estimate_block_tokens(block);
                image_count += 1;
            }
            _ => user_text += estimate_block_tokens(block),
        }
    }
    push_seg(&mut segments, LABEL_USER_MESSAGE, user_text, None);
    push_seg(&mut segments, LABEL_USER_IMAGES, image_tokens, (image_count > 0).then_some(image_count));

    ContextAnatomy {
        segments,
        system_stable_hash: ctx
            .system_parts
            .as_ref()
            .map(|p| p.stable_hash())
            .unwrap_or_default(),
        os_stable_hash: ctx.os_stable_hash.clone().unwrap_or_default(),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::pipeline::PipelineContext;
    use crate::context::system_prompt::SystemPromptParts;
    use crate::infra::protocol::{ContentBlock, ToolDef};
    use serde_json::json;

    /// 与 agent_profile_migration.rs content_hash 的黄金向量交叉验证：
    /// 相同输入必须产出相同 12 hex（两处实现同源，漂移即错）。
    #[test]
    fn fnv_matches_migration_content_hash_shape() {
        // 手工复算 migration 版逻辑（四字段 [provider, model, url, key]）
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for part in ["glm", "glm-5.3", "https://api", "sk-key"] {
            for b in part.as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            h ^= 0x1f;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!(fnv1a_12hex(&["glm", "glm-5.3", "https://api", "sk-key"]), format!("{h:012x}"));
    }

    #[test]
    fn fnv_separator_byte_disambiguates_adjacent_parts() {
        assert_ne!(fnv1a_12hex(&["ab", "c"]), fnv1a_12hex(&["a", "bc"]));
        assert_ne!(fnv1a_12hex(&["a", ""]), fnv1a_12hex(&["", "a"])); // 位置敏感
        assert_eq!(fnv1a_12hex(&[]), format!("{:012x}", 0xcbf2_9ce4_8422_2325u64));
    }

    fn tool_def(name: &str, desc: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn hash_tool_defs_is_order_and_content_sensitive() {
        let a = [tool_def("b_tool", "d2"), tool_def("a_tool", "d1")];
        let b = [tool_def("a_tool", "d1"), tool_def("b_tool", "d2")];
        assert_ne!(hash_tool_defs(&a), hash_tool_defs(&b), "顺序变化须可见");
        let c = [tool_def("a_tool", "d1-改"), tool_def("b_tool", "d2")];
        assert_ne!(hash_tool_defs(&b), hash_tool_defs(&c));
        let d = [tool_def("a_tool", "d1"), tool_def("b_tool", "d2")];
        assert_eq!(hash_tool_defs(&b), hash_tool_defs(&d));
        assert_eq!(hash_tool_defs(&[]), format!("{:012x}", 0xcbf2_9ce4_8422_2325u64));
    }

    /// anatomy 聚合不查库——一个裸 in-memory pool 即可（无 migration 无种子）。
    async fn make_ctx() -> PipelineContext {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        let agent = crate::db::models::AgentRow {
            id: "a-t".into(),
            name: "t".into(),
            provider: "anthropic".into(),
            model: "claude".into(),
            system_prompt: String::new(),
            api_key_ref: "vault://t".into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: "{}".into(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            supports_vision: 0,
            description: String::new(),
            avatar: None,
            workspace_path: None,
            model_profile_id: None,
            fallback_profile_ids: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        PipelineContext::new(
            pool,
            agent,
            None,
            Vec::new(),
            Vec::new(),
            true,
            None,
            Vec::new(),
            crate::context::token::ContextBudget {
                max_input_tokens: 100_000,
            },
            "conv-test".into(),
            crate::infra::cancel::CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn build_anatomy_segments_and_hashes() {
        let mut ctx = make_ctx().await;
        let mut parts = SystemPromptParts::build(None, "你是一个助手", true, "OS: Windows");
        parts.delegation_hint = Some("可调度 agent 清单".into());
        parts.word_style = Some("## Word 文档样式偏好\n\n宋体".into());
        ctx.system_parts = Some(parts);
        ctx.os_stable_hash = Some("abc123".into());
        ctx.summary = Some("此前对话的滚动摘要".into());
        ctx.history_messages = vec![
            crate::infra::protocol::ChatMessage::from_text("user", "历史问题历史问题历史问题"),
            crate::infra::protocol::ChatMessage::from_text("assistant", "历史回答"),
        ];
        ctx.final_blocks = vec![
            ContentBlock::text("今天的正题"),
            ContentBlock::image("aGVsbG8gd29ybGQ=", "image/png"), // 解码 11 字节 → 85 floor
        ];

        let anatomy = build_anatomy(&ctx);
        let labels: Vec<&str> = anatomy.segments.iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            vec![
                "system_persona",
                "system_tool_hint",
                "system_os_context",
                "system_delegation_hint",
                "system_word_style",
                "summary",
                "history",
                "user_message",
                "user_images",
            ]
        );
        let history = anatomy.segments.iter().find(|s| s.label == "history").unwrap();
        assert_eq!(history.count, Some(2));
        let images = anatomy.segments.iter().find(|s| s.label == "user_images").unwrap();
        assert_eq!(images.count, Some(1));
        assert_eq!(images.est, 85); // IMAGE_TOKEN_FLOOR
        assert!(!anatomy.system_stable_hash.is_empty());
        assert_eq!(anatomy.os_stable_hash, "abc123");
    }

    #[tokio::test]
    async fn build_anatomy_omits_empty_segments_and_parts_none() {
        // system_parts = None（散落测试构造）：system 五段全跳过
        let ctx = make_ctx().await;
        let anatomy = build_anatomy(&ctx);
        assert!(anatomy.segments.iter().all(|s| !s.label.starts_with("system_")));
        assert!(anatomy.segments.iter().all(|s| s.est > 0), "est=0 段必须省略");
        assert_eq!(anatomy.system_stable_hash, "");
        assert_eq!(anatomy.os_stable_hash, "");
    }
}
