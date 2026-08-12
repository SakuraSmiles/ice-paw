//! 统一模态适配层 —— 把"图片给不给模型看 / 给不了怎么办"收敛到一处（事2 / 方案 C）。
//!
//! ## 背景：4 个图片入口原本各走各的路
//! 项目里有 4 处会把 `Image` 块塞进送给 LLM 的上下文：
//! ① 用户上传/粘贴图片（`chat_cmd::send_message` → `final_blocks` → Pipeline）
//! ② 工具回传 `image_png`（`tool_executor` 注入 `Image` 块）
//! ③ 历史消息里的 `Image` 块（`HistoryStage` 从 DB 还原）
//! ④ `view_attachment_image` 工具（扫描件/图片型 PDF 渲染代读）
//!
//! 改造前只有 ④ 接了多级 fallback（`vision.rs` 的 `from_prefs` / `from_agent` /
//! `from_mcp_env`），①②③ 一律原样塞 `Image` → 遇到不支持视觉的模型要么 400，要么
//! 模型"声称看不到"（即本次生产反馈的 agent 行为）。本模块提供统一适配，供 4 个入口共用。
//!
//! ## 两个核心函数
//! - [`gather_vision_candidates`]：从 DB 收集可用视觉凭据（按优先级：显式 vision 配置 →
//!   agent 自带视觉模型 → GLM 视觉 MCP env）。把原本散在 `view_attachment_image` 里的
//!   收集逻辑抽公共，4 个入口用同一份。
//! - [`adapt_blocks_for_vision`]：按目标模型"有效视觉能力"适配 blocks——支持视觉则原样过；
//!   否则逐图用候选凭据代读（OCR）成 `Text`，代读不了的剥离 + 诚实提示（绝不伪造）。
//!
//! "有效视觉能力"由 [`crate::harness::provider::model_info::effective_supports_vision`]
//! 决定（agent 显式 `supports_vision=1` **或** 模型表自动探测，OR 关系、零 schema 改动）。
//!
//! ## 与事1（`build_modality_hint`）的协作
//! 事1 给视觉 agent 注入"你已收到 N 张图片、无需调工具"的元提示，消除"看到了却说没看到"
//! 的认知错位。本层补另一半：非视觉 agent 的图片走代读/诚实剥离。两者按 `effective_vision`
//! 分工——视觉走事1 元提示 + 原图直送；非视觉走本层代读/剥离（Phase 3 接线时统一到 Stage）。

use sqlx::SqlitePool;

use crate::db::repo;
use crate::db::models::AgentRow;
use crate::harness::vision::{self, VisionCredential};
use crate::infra::protocol::ContentBlock;

/// 适配结果——便于调用方做 tracing、注入差异化元提示（Phase 3 Stage 消费）。
#[derive(Debug, Clone, Default)]
pub struct AdaptOutcome {
    /// 适配后的 blocks（可能含代读出的 Text 块 / 诚实提示块）。
    pub blocks: Vec<ContentBlock>,
    /// OCR 成功替换掉的图片数（`effective_vision=false` 但凭据代读成功）。
    pub ocr_replaced: usize,
    /// 因无凭据 / 全部凭据失败而被剥离的图片数（诚实提示用）。
    pub dropped: usize,
}

impl AdaptOutcome {
    /// 本次适配是否实际改变了 blocks（有图被代读或剥离）。纯 passthrough 时为 false。
    pub fn changed(&self) -> bool {
        self.ocr_replaced > 0 || self.dropped > 0
    }
}

/// 从 DB 收集可用视觉凭据，按优先级排序（多级 fallback 的候选清单）。
///
/// 顺序与原 `view_attachment_image` 工具一致（[`crate::harness::vision`] 模块头注释）：
/// ① 显式 vision 配置（用户在「设置-视觉读取」配的——精确 model/url，最高优先级）
/// ② 当前 agent 自己的凭据（GLM→glm-4v / OpenAI→gpt-4o / MiniMax→M3，用 agent 的 key）
/// ③ 已配的 GLM 视觉 MCP 的 env（`Z_AI_API_KEY`→glm-4v，零配置兜底第二环）
///
/// `api_key` 为已解析的 agent 明文 key（DB 只存引用槽位，由调用方解析后传入）。
/// 任一级 DB 查询失败仅 warn 跳过——降级到能拿到的凭据，不阻塞主流程。
pub async fn gather_vision_candidates(
    pool: &SqlitePool,
    agent: &AgentRow,
    api_key: Option<&str>,
) -> Vec<VisionCredential> {
    let mut candidates: Vec<VisionCredential> = Vec::new();

    // ① 显式 vision 配置
    match repo::preferences::get_all(pool).await {
        Ok(prefs) => {
            if let Some(c) = vision::from_prefs(&prefs) {
                candidates.push(c);
            }
        }
        Err(e) => tracing::warn!(
            target: "ice_paw.modal",
            err = %e, "读取 preferences 失败，跳过 vision 配置凭据"
        ),
    }

    // ② 当前 agent 自带视觉模型
    if let Some(key) = api_key.filter(|s| !s.is_empty()) {
        if let Some(c) = vision::from_agent(&agent.provider, key) {
            candidates.push(c);
        }
    }

    // ③ GLM 视觉 MCP env
    match repo::mcp_server::list_all(pool).await {
        Ok(servers) => {
            for s in servers.iter().filter(|s| s.enabled) {
                if let Some(c) = vision::from_mcp_env(&s.env) {
                    candidates.push(c);
                    break; // 同质 GLM key，取第一个即够
                }
            }
        }
        Err(e) => tracing::warn!(
            target: "ice_paw.modal",
            err = %e, "列出 MCP server 失败，跳过 MCP env 凭据"
        ),
    }

    candidates
}

/// 按"目标模型是否支持视觉"统一适配 blocks（4 个图片入口共用）。
///
/// - `effective_vision=true`：原样返回（模型直接看图，高保真、零额外 LLM 调用）。
/// - `effective_vision=false`：逐个 `Image` 块用 `candidates` 代读（[`vision::describe_image`]）；
///   成功 → 替换成 `Text`（OCR 文本）；失败（无凭据 / 全失败 / base64 损坏）→ 剥离并计数。
///   末尾若有剥离，注入一条诚实提示 `Text` 块（告知"有 N 张图读不了 + 如何解决"）。
///
/// **幂等可重入**：历史 blocks 里已经代读成的 `Text` 不再含 `Image`，二次进入直接 passthrough，
/// 不会重复 OCR（Phase 3 历史路径每轮调用安全）。代读失败的图每轮会被剥离+提示——这是诚实
/// 语义的代价；如需避免重复提示，未来可在 DB 侧缓存代读结果（当前 YAGNI）。
///
/// 网络错误被内部吸收（计入 dropped），**绝不向上抛**——视觉适配失败不应中断主对话循环。
pub async fn adapt_blocks_for_vision(
    blocks: &[ContentBlock],
    effective_vision: bool,
    candidates: &[VisionCredential],
) -> AdaptOutcome {
    // 视觉模型：原样过（绝大多数 agent 走这条，零开销）。
    if effective_vision {
        return AdaptOutcome {
            blocks: blocks.to_vec(),
            ocr_replaced: 0,
            dropped: 0,
        };
    }

    let mut out: Vec<ContentBlock> = Vec::with_capacity(blocks.len() + 1);
    let mut ocr_replaced = 0usize;
    let mut dropped = 0usize;

    for b in blocks {
        if let ContentBlock::Image { data, media_type } = b {
            match ocr_image(data, media_type, candidates).await {
                Some(text) => {
                    ocr_replaced += 1;
                    out.push(ContentBlock::text(format!(
                        "[图片经视觉凭据代读为文本]\n{text}"
                    )));
                }
                None => dropped += 1,
            }
        } else {
            out.push(b.clone());
        }
    }

    if dropped > 0 {
        out.push(ContentBlock::text(format!(
            "[系统提示：本次消息含 {dropped} 张图片，但当前模型不支持视觉，且无可用视觉凭据\
             （视觉配置 / agent 自带视觉模型 / GLM 视觉 MCP）代为读取。如需识别图片内容，\
             请在「设置-视觉读取」配置视觉模型，或换用支持视觉的 agent。]"
        )));
    }

    AdaptOutcome {
        blocks: out,
        ocr_replaced,
        dropped,
    }
}

/// 用候选凭据逐个试 [`vision::VisionCredential::describe`]；首个成功返回文本，全失败 / 无候选 /
/// base64 解码失败返回 `None`。网络/凭据错误不向上抛（调用方计入 dropped 走诚实剥离）。
async fn ocr_image(
    data_base64: &str,
    media_type: &str,
    candidates: &[VisionCredential],
) -> Option<String> {
    use base64::Engine as _;

    if candidates.is_empty() {
        return None;
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64)
        .map_err(|e| {
            tracing::warn!(
                target: "ice_paw.modal",
                err = %e, "Image 块 base64 解码失败，跳过代读"
            );
            e
        })
        .ok()?;

    let mut last_err: Option<String> = None;
    for cred in candidates {
        match cred.describe(&bytes, media_type).await {
            Ok(text) => {
                tracing::info!(
                    target: "ice_paw.modal",
                    source = %cred.source,
                    chars = text.len(),
                    "视觉凭据代读图片成功"
                );
                return Some(text);
            }
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.modal",
                    source = %cred.source,
                    err = %e,
                    "视觉凭据代读失败，尝试下一级"
                );
                last_err = Some(format!("{}: {e}", cred.source));
            }
        }
    }

    tracing::warn!(
        target: "ice_paw.modal",
        tried = ?candidates.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(),
        last_err = ?last_err,
        "所有视觉凭据代读均失败"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(data: &str, mt: &str) -> ContentBlock {
        ContentBlock::Image {
            data: data.into(),
            media_type: mt.into(),
        }
    }

    // ---- effective_vision=true：passthrough ----

    #[tokio::test]
    async fn passthrough_when_effective_vision_true() {
        let blocks = vec![
            ContentBlock::text("hi"),
            img("AAAA", "image/png"),
            img("BBBB", "image/jpeg"),
        ];
        // 视觉模型 + 空凭据清单：仍原样过（不触发代读）。
        let out = adapt_blocks_for_vision(&blocks, true, &[]).await;
        assert_eq!(out.blocks.len(), 3, "passthrough 应保持块数");
        // 两张图原样保留（ContentBlock 无 PartialEq，按位置/类型断言）
        assert!(out.blocks[1].is_image());
        assert!(out.blocks[2].is_image());
        assert_eq!(out.ocr_replaced, 0);
        assert_eq!(out.dropped, 0);
        assert!(!out.changed());
    }

    // ---- effective_vision=false + 无凭据：剥离 + 诚实提示 ----

    #[tokio::test]
    async fn non_vision_drops_images_and_injects_honest_hint_when_no_credentials() {
        let blocks = vec![
            ContentBlock::text("看这张图"),
            img("AAAA", "image/png"),
            img("BBBB", "image/jpeg"),
        ];
        let out = adapt_blocks_for_vision(&blocks, false, &[]).await;
        // 两张图都被剥离
        assert_eq!(out.dropped, 2);
        assert_eq!(out.ocr_replaced, 0);
        assert!(out.changed());
        // 文本块保留，无 Image 残留
        assert!(out.blocks.iter().all(|b| !b.is_image()));
        assert_eq!(out.blocks[0].as_text(), Some("看这张图"));
        // 末尾诚实提示含图片数 2 + 引导用户配置
        let hint = out.blocks.last().unwrap().as_text().unwrap();
        assert!(hint.contains("2 张图片"), "提示应含被剥离图片数，实际: {hint}");
        assert!(hint.contains("视觉读取") || hint.contains("视觉模型"));
    }

    #[tokio::test]
    async fn non_vision_single_image_dropped_hint_count_is_one() {
        let blocks = vec![img("AAAA", "image/png")];
        let out = adapt_blocks_for_vision(&blocks, false, &[]).await;
        assert_eq!(out.dropped, 1);
        let hint = out.blocks.last().unwrap().as_text().unwrap();
        assert!(hint.contains("1 张图片"), "单图剥离提示计数应为 1，实际: {hint}");
    }

    // ---- 混合块：非图片块原样保留 ----

    #[tokio::test]
    async fn non_image_blocks_preserved_in_order() {
        let blocks = vec![
            ContentBlock::text("第一段"),
            img("AAAA", "image/png"),
            ContentBlock::text("第二段"),
        ];
        let out = adapt_blocks_for_vision(&blocks, false, &[]).await;
        // 两个文本块顺序保留，中间图被剥，末尾加提示
        assert_eq!(out.blocks[0].as_text(), Some("第一段"));
        assert_eq!(out.blocks[1].as_text(), Some("第二段"));
        assert_eq!(out.dropped, 1);
        // 最后一块是诚实提示
        assert!(out.blocks.last().unwrap().as_text().unwrap().contains("1 张图片"));
    }

    // ---- 纯文本无图：非视觉也不注入多余提示 ----

    #[tokio::test]
    async fn non_vision_no_images_no_hint_injected() {
        let blocks = vec![ContentBlock::text("纯文本消息")];
        let out = adapt_blocks_for_vision(&blocks, false, &[]).await;
        assert_eq!(out.blocks.len(), 1);
        assert_eq!(out.blocks[0].as_text(), Some("纯文本消息"));
        assert_eq!(out.dropped, 0);
        assert!(!out.changed());
    }

    // ---- base64 损坏：计入 dropped，不抛错 ----

    #[tokio::test]
    async fn malformed_base64_counted_as_dropped_not_panicked() {
        // base64 引擎会拒绝非法字符 @@@
        let blocks = vec![img("@@@不是合法base64@@@", "image/png")];
        // 即使有候选（空清单里没法验，但解码在 describe 之前发生）：
        // 这里用空候选先验 dropped 计数路径。
        let out = adapt_blocks_for_vision(&blocks, false, &[]).await;
        assert_eq!(out.dropped, 1);
        assert!(out.blocks.last().unwrap().as_text().unwrap().contains("1 张图片"));
    }
}
