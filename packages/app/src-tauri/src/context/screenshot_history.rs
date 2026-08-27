//! 截图历史压缩 —— computer use 的上下文体积治理（阶段一·看屏3）。
//!
//! **问题**：computer use 循环里 agent 每轮截一张图（1600 长边 PNG，几百 KB
//! base64），工具结果消息进 `ctx.messages` 后**每轮全量重发**——40 轮截图循环
//! = 第 40 轮重发 39 张旧图，上下文 O(n²) 膨胀，几轮就顶满窗口。
//!
//! **解法**：只保留**最近 K 张**工具截图，更早的替换成一条诚实的 Text marker
//! （不撒谎——[`crate::harness::modal::strip_image_blocks_to_marker`] 的文案
//! 说「无视觉能力」在这里是假的，不复用）。双钩：
//! - **钩 A（轮内）**：`loop_engine` 工具结果 push 后对本轮 in-flight
//!   `ctx.messages` 压缩——长工具循环的治理主力；
//! - **钩 B（跨回合）**：[`ScreenshotHistoryStage`] 在 HistoryStage 之后、
//!   MemoryStage/TokenWindowStage 之前对 DB 回灌的历史压缩——否则下一回合
//!   全量图照旧回灌。
//!
//! **边界（append-only 不变式无违反）**：只动 LLM 视图（in-flight / pipeline
//! 的消息克隆），DB 行与 session-events 保完整图——回放/审计仍无损。
//!
//! **识别判据**：工具截图 = **含 ToolResult 块的 user 消息**里的 Image 块
//! （`tool_executor` 把工具返图注入 tool_result 同消息）。用户上传图
//! （无 ToolResult 的 user 消息）不受影响。

use async_trait::async_trait;

use crate::context::pipeline::{PipelineContext, PipelineStage};
use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock};

/// 保留的最近工具截图张数。
///
/// 3 的依据：computer use 循环典型形态是「截图→定位→操作→验证」，验证轮
/// 需要对比操作前后两张图，3 张给「操作前/操作后/再验证」留满余量；更早的
/// 画面定位信息已沉淀在 assistant 文本里，图本身不再有增量价值。
pub(crate) const SCREENSHOT_KEEP_LAST_K: usize = 3;

/// 压缩消息列表里的工具截图：保留最近 `keep_last_k` 张 Image 块
///（含 ToolResult 块的 user 消息里的），其余整块替换为单条诚实 marker。
///
/// 倒序消耗配额；单条消息内图数超剩余配额时保留**靠后的**（同一轮工具结果
/// 里靠后 = 较新）。返回被压缩掉的 Image 块总数（0 = 无事发生）。
pub(crate) fn compact_screenshot_history(
    messages: &mut [ChatMessage],
    keep_last_k: usize,
) -> usize {
    let mut remaining = keep_last_k;
    let mut compacted = 0usize;
    for msg in messages.iter_mut().rev() {
        // 只处理工具结果消息（识别判据见模块注释）；其余消息的图（用户上传）
        // 是用户亲自给的内容，不属本治理范围。
        if !msg
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        {
            continue;
        }
        let image_count = msg
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image { .. }))
            .count();
        if image_count == 0 {
            continue;
        }
        if remaining >= image_count {
            remaining -= image_count;
            continue;
        }
        // 配额不足：本条保留靠后 `remaining` 张（较新），其余折成一条 marker
        //（marker 落在首个被压缩图的原位，块序稳定）。
        let keep_from = image_count - remaining; // 正序第 keep_from 张起保留
        let mut img_idx = 0usize;
        let mut marker_emitted = false;
        let mut new_content: Vec<ContentBlock> = Vec::with_capacity(msg.content.len());
        for b in msg.content.iter() {
            if matches!(b, ContentBlock::Image { .. }) {
                if img_idx >= keep_from {
                    new_content.push(b.clone());
                } else if !marker_emitted {
                    new_content.push(ContentBlock::text(marker_text(
                        keep_from,
                        keep_last_k,
                    )));
                    marker_emitted = true;
                }
                img_idx += 1;
            } else {
                new_content.push(b.clone());
            }
        }
        compacted += keep_from;
        msg.content = new_content;
        // 配额已耗尽——更老消息里的工具图全压（勿把残留配额复用给更老消息，
        // 否则保留总数会超 K）。
        remaining = 0;
    }
    compacted
}

/// 诚实 marker 文案：说明省略原因 + 怎么拿回来，不伪造能力状态。
fn marker_text(omitted: usize, keep_last_k: usize) -> String {
    format!(
        "[{omitted} 张更早的工具截图未随本条发送——为控制上下文体积，仅保留最近 \
         {keep_last_k} 张。需要画面时重新调用截图工具获取当前状态。]"
    )
}

// =========================================================================
// Stage（钩 B：跨回合，DB 回灌历史压缩）
// =========================================================================

/// Stage 4.6：压缩历史里的工具截图（最近 K 张之外替换为 marker）。
///
/// 位置：[`crate::context::stages::ToolFailureFoldStage`] 之后、
/// [`crate::context::memory::MemoryStage`] / [`crate::context::stages::TokenWindowStage`]
/// 之前——摘要与 token 裁剪看到的都已是压缩后体积（否则体积治理晚了一拍）。
/// 只读 `ctx.history_messages`（LLM 视图），不触碰 DB。
pub(crate) struct ScreenshotHistoryStage;

#[async_trait]
impl PipelineStage for ScreenshotHistoryStage {
    fn name(&self) -> &'static str {
        "screenshot_history"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let compacted =
            compact_screenshot_history(&mut ctx.history_messages, SCREENSHOT_KEEP_LAST_K);
        if compacted > 0 {
            tracing::info!(
                target: "ice_paw.context",
                compacted,
                keep_last_k = SCREENSHOT_KEEP_LAST_K,
                "ScreenshotHistoryStage: 历史工具截图已压缩（LLM 视图；DB/事件日志保完整图）"
            );
        }
        Ok(())
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_img_msg(n_img: usize) -> ChatMessage {
        let mut content = vec![ContentBlock::ToolResult {
            tool_use_id: "tu-1".into(),
            content: "{\"ok\":true}".into(),
            is_error: None,
        }];
        for _ in 0..n_img {
            content.push(ContentBlock::Image {
                data: "AAAA".into(),
                media_type: "image/png".into(),
            });
        }
        ChatMessage {
            role: "user".into(),
            content,
            source_rowid: None,
            source_seq: None,
        }
    }

    fn user_img_msg() -> ChatMessage {
        ChatMessage {
            role: "user".into(),
            content: vec![
                ContentBlock::text("看这张图"),
                ContentBlock::Image {
                    data: "BBBB".into(),
                    media_type: "image/png".into(),
                },
            ],
            source_rowid: None,
            source_seq: None,
        }
    }

    fn count_images(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .map(|m| m.content.iter().filter(|b| matches!(b, ContentBlock::Image { .. })).count())
            .sum()
    }

    #[test]
    fn keeps_last_k_compacts_older() {
        let mut msgs: Vec<ChatMessage> = (0..5).map(|_| tool_img_msg(1)).collect();
        let compacted = compact_screenshot_history(&mut msgs, 3);
        assert_eq!(compacted, 2);
        assert_eq!(count_images(&msgs), 3);
        // 保留的是靠后（较新）的 3 条；前 2 条各有一条 marker 且 ToolResult 原样
        for (i, m) in msgs.iter().enumerate() {
            assert!(
                m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. })),
                "ToolResult 块不得被压缩掉（{i}）"
            );
            if i < 2 {
                assert!(m
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("未随本条发送"))));
            }
        }
    }

    #[test]
    fn partial_within_single_message_keeps_newer_images() {
        // 单条 4 图、配额 3 → 保留靠后 3 张，压 1 张
        let mut msgs = vec![tool_img_msg(4)];
        let compacted = compact_screenshot_history(&mut msgs, 3);
        assert_eq!(compacted, 1);
        assert_eq!(count_images(&msgs), 3);
        // marker 在块序首位被压缩图原位（ToolResult 之后、保留图之前）
        let m = &msgs[0];
        assert!(matches!(m.content[0], ContentBlock::ToolResult { .. }));
        assert!(matches!(m.content[1], ContentBlock::Text { .. }));
        assert_eq!(
            m.content.iter().filter(|b| matches!(b, ContentBlock::Image { .. })).count(),
            3
        );
    }

    #[test]
    fn user_uploaded_images_untouched() {
        let mut msgs = vec![
            tool_img_msg(2),
            user_img_msg(),
            tool_img_msg(2),
            tool_img_msg(2),
        ];
        // 配额 3：最新两条工具消息共 4 图 → 最新条保 2、次新条保 1、最老条全压；
        // 用户上传图（无 ToolResult）不受影响
        let compacted = compact_screenshot_history(&mut msgs, 3);
        assert_eq!(compacted, 3);
        assert_eq!(count_images(&msgs), 4); // 3 张工具图 + 1 张用户图
        let user_msg = &msgs[1];
        assert_eq!(
            user_msg
                .content
                .iter()
                .filter(|b| matches!(b, ContentBlock::Image { .. }))
                .count(),
            1
        );
        assert!(user_msg
            .content
            .iter()
            .all(|b| !matches!(b, ContentBlock::Text { text } if text.contains("未随本条发送"))));
    }

    #[test]
    fn zero_quota_compacts_all_and_none_is_noop() {
        // 配额 0 = 全压（未来若开放为 agent 旋钮的语义地基）
        let mut msgs = vec![tool_img_msg(2)];
        assert_eq!(compact_screenshot_history(&mut msgs, 0), 2);
        assert_eq!(count_images(&msgs), 0);

        // 无图 / 空表 = 0 改动
        let mut no_img = vec![ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "t".into(),
                content: "text-only".into(),
                is_error: None,
            }],
            source_rowid: None,
            source_seq: None,
        }];
        assert_eq!(compact_screenshot_history(&mut no_img, 3), 0);
    }

    #[tokio::test]
    async fn stage_compacts_pipeline_history_messages() {
        let pool = crate::context::pipeline_tests::fresh_pool().await;
        let ctx_base = crate::context::pipeline_tests::make_ctx(
            pool,
            crate::context::pipeline_tests::make_agent(),
            None,
            Vec::new(),
            Vec::new(),
            true,
        );
        let mut ctx = ctx_base;
        ctx.history_messages = (0..5).map(|_| tool_img_msg(1)).collect();
        ScreenshotHistoryStage.execute(&mut ctx).await.unwrap();
        assert_eq!(count_images(&ctx.history_messages), 3);
    }
}
