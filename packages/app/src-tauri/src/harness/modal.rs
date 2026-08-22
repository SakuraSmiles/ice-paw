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

use crate::db::models::AgentRow;
use crate::db::repo;
use crate::harness::error_mapping::{classify_llm_error, LlmErrorKind};
use crate::harness::vision::{self, VisionCredential};
use crate::infra::protocol::ContentBlock;

/// OCR 进度回调签名——每张图 OCR 完成（成功或失败）后触发一次。
///
/// 参数：`(done, total)` —— `done` 是 1-based「第几张完成」，`total` 是本批
/// 图片总数。`done == total` 时调用方知道本批结束。
///
/// 用途：让调用方（如 `ModalCapabilityStage`）emit `chat:processing` 心跳，
/// 把 OCR 真实进度透传给前端，撑住 60s 静默超时窗口（多图串行 OCR 易超 60s）。
///
/// 设计取舍：本函数保持纯函数语义——回调为外部注入的 `Fn`，不依赖 emitter
/// 类型，调用方决定要不要 emit（测试 / 工具返图等单图场景传 None）。
pub type ProgressCb<'a> = &'a (dyn Fn(u32, u32) + Send + Sync);

/// 适配结果——便于调用方做 tracing、注入差异化元提示（Phase 3 Stage 消费）。
#[derive(Debug, Clone, Default)]
pub struct AdaptOutcome {
    /// 适配后的 blocks（可能含代读出的 Text 块 / 诚实提示块）。
    pub blocks: Vec<ContentBlock>,
    /// OCR 成功替换掉的图片数（`effective_vision=false` 但凭据代读成功）。
    pub ocr_replaced: usize,
    /// 因无凭据 / 全部凭据失败而被剥离的图片数（诚实提示用）。
    pub dropped: usize,
    /// 逐图明细（session-events `modal_adapted` 用）：原始 blocks 下标 → 处置 +
    /// 代读全文。「Model-visible means logged」——代读文本是模型真实消费的内容。
    pub items: Vec<crate::harness::event_log::ModalAdaptedItem>,
    /// 最后一张被剥离图片的失败原因分类（驱动诚实提示文案分支，缺口③）。
    ///
    /// - `None`：未发起有效调用（无候选凭据 / base64 解码失败）→ 提示引导用户**配置**
    ///   视觉读取。
    /// - `Some(kind)`：已有候选凭据但全部 `describe` 调用失败 → 提示按 kind 给**具体原因**
    ///   （敏感拒 / 限流 / 密钥错 / 网络），避免笼统的「无凭据」误导用户去查配置。
    pub drop_reason: Option<LlmErrorKind>,
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
    on_progress: Option<ProgressCb<'_>>,
) -> AdaptOutcome {
    // 视觉模型：原样过（绝大多数 agent 走这条，零开销）。
    if effective_vision {
        return AdaptOutcome {
            blocks: blocks.to_vec(),
            ocr_replaced: 0,
            dropped: 0,
            items: Vec::new(),
            drop_reason: None,
        };
    }

    let mut out: Vec<ContentBlock> = Vec::with_capacity(blocks.len() + 1);
    let mut ocr_replaced = 0usize;
    let mut dropped = 0usize;
    // 逐图明细（modal_adapted 事件用，见 AdaptOutcome::items）。
    let mut items: Vec<crate::harness::event_log::ModalAdaptedItem> = Vec::new();
    // 记录最后一张被剥离图的失败原因，驱动诚实提示文案分支（缺口③）。
    // Some 优先：只要任一图是「有凭据但调用失败」(Some)，就不用「无凭据」(None) 文案。
    let mut last_drop_reason: Option<LlmErrorKind> = None;
    // OCR 总数：本批 Image 块总数（不计非图块），用于进度回调的 total 字段。
    // 计算一次避免回调内重复 len()——blocks 通常很短，但 ProgressCb 在
    // ModalCapabilityStage 内可能 emit chat:processing，序列化 O(1) 仍宜最小化。
    let total_images: u32 = blocks.iter().filter(|b| b.is_image()).count() as u32;
    let mut done_images: u32 = 0;

    for (index, b) in blocks.iter().enumerate() {
        if let ContentBlock::Image { data, media_type } = b {
            let result = ocr_image(data, media_type, candidates).await;
            match result {
                Ok(text) => {
                    ocr_replaced += 1;
                    out.push(ContentBlock::text(format!(
                        "[图片经视觉凭据代读为文本]\n{text}"
                    )));
                    items.push(crate::harness::event_log::ModalAdaptedItem {
                        index,
                        outcome: "substituted".into(),
                        ocr_text: Some(text),
                    });
                }
                Err(reason) => {
                    dropped += 1;
                    items.push(crate::harness::event_log::ModalAdaptedItem {
                        index,
                        outcome: "dropped".into(),
                        ocr_text: None,
                    });
                    // 多图聚合：当前 Some（有凭据但调用失败）按 prefer() 与历史合并——
                    // Sensitive（输入判定）优先于凭据级瞬态错误，否则保留首个；
                    // 当前 None（无候选）不覆盖已有 Some。与 ocr_image 单图内逻辑同源。
                    last_drop_reason = match reason {
                        Some(k) => LlmErrorKind::prefer(last_drop_reason, k),
                        None => last_drop_reason,
                    };
                }
            }
            // 每张图完成（成功或失败都算 done）即回调——失败也算进度推进。
            // total_images == 0 时回调永不触发（视觉模型早返回），无需空批次兜底。
            done_images += 1;
            if let Some(cb) = on_progress {
                cb(done_images, total_images);
            }
        } else {
            out.push(b.clone());
        }
    }

    // 诚实提示按失败原因分支，避免笼统「无凭据」误导（缺口③）。
    if let Some(hint) = dropped_hint(dropped, last_drop_reason) {
        out.push(ContentBlock::text(hint));
    }

    AdaptOutcome {
        blocks: out,
        ocr_replaced,
        dropped,
        items,
        drop_reason: last_drop_reason,
    }
}

/// 生成「图片被剥离」的诚实提示文本；`dropped = 0` 时返回 `None`（不注入多余提示）。
///
/// **按失败原因分支**（[`LlmErrorKind`]），避免笼统的「无可用凭据」文案误导用户——
/// 用户明明配了视觉凭据，却被提示「无凭据」会困惑地去查配置（其实配置没问题，是调用被拒）。
/// - `drop_reason = None`：未发起有效调用（无候选凭据 / base64 损坏）→ 提示引导用户**配置**
///   视觉读取或换视觉 agent。
/// - `drop_reason = Some(kind)`：已有凭据但全部 `describe` 调用失败 → 按 kind 给**具体原因**
///   （敏感拒 / 限流 / 密钥错 / 网络），让用户知道问题在调用结果而非「没配置」。
///
/// 抽成纯函数便于单测各原因分支文案（adapt 集成路径需 mock describe，单测覆盖成本高）。
pub(crate) fn dropped_hint(dropped: usize, drop_reason: Option<LlmErrorKind>) -> Option<String> {
    if dropped == 0 {
        return None;
    }
    let hint = match drop_reason {
        None => format!(
            "[系统提示：本次消息含 {dropped} 张图片，但当前模型不支持视觉，且未配置可用的视觉\
             读取凭据（视觉配置 / agent 自带视觉模型 / GLM 视觉 MCP）。如需识别图片内容，\
             请在「设置-视觉读取」配置视觉模型，或换用支持视觉的 agent。]"
        ),
        Some(LlmErrorKind::Unknown) => format!(
            "[系统提示：本次消息含 {dropped} 张图片，当前模型不支持视觉，已尝试用可用视觉凭据\
             代读但未能识别。如需识别，请换用支持视觉的 agent，或在「设置-视觉读取」\
             检查视觉模型配置。]"
        ),
        // 已知分类（敏感/限流/鉴权/超长/网络）：friendly_text 非空，给出真实原因。
        Some(kind) => format!(
            "[系统提示：本次消息含 {dropped} 张图片，当前模型不支持视觉，已尝试用可用视觉\
             凭据代读，但{}。]",
            kind.friendly_text()
        ),
    };
    Some(hint)
}

/// 把历史消息里的 `Image` 块**剥成一个诚实 marker**（非视觉 agent 历史路径专用）。
///
/// 与 [`adapt_blocks_for_vision`] 的区别：历史图片每轮重新 OCR 成本高（N 图 × M 轮 = N×M
/// 次视觉调用）且代读文本不落库（下轮还得重来），故历史路径**不 OCR、只剥离**——每条消息
/// 不论含几张图，只插**一条** marker（"曾含 N 张图、当前模型无视觉、已省略"），避免重复
/// 提示噪声。当前轮的图走 [`adapt_blocks_for_vision`] 完整代读（用户此刻就想让 agent 看到）。
///
/// 无图消息原样返回（零开销，绝大多数历史消息走这条）。
pub fn strip_image_blocks_to_marker(blocks: &[ContentBlock]) -> Vec<ContentBlock> {
    let image_count = blocks.iter().filter(|b| b.is_image()).count();
    if image_count == 0 {
        return blocks.to_vec();
    }
    let mut out: Vec<ContentBlock> = Vec::with_capacity(blocks.len());
    let mut marker_inserted = false;
    for b in blocks {
        if b.is_image() {
            if !marker_inserted {
                out.push(ContentBlock::text(format!(
                    "[此历史消息曾含 {image_count} 张图片；当前模型无视觉能力，已省略图片内容。\
                     如需识别，请在「设置-视觉读取」配置视觉模型或换用视觉 agent。]"
                )));
                marker_inserted = true;
            }
        } else {
            out.push(b.clone());
        }
    }
    out
}

/// 用候选凭据逐个试 [`vision::VisionCredential::describe`]；首个成功返回其文本。
///
/// 返回 `Result<String, Option<LlmErrorKind>>`：
/// - `Ok(text)` —— 代读成功。
/// - `Err(None)` —— 未发起有效调用（候选为空 / base64 解码失败）。
/// - `Err(Some(kind))` —— 已有候选但全部 `describe` 调用失败，`kind` 为按
///   [`LlmErrorKind::prefer`] 选出的**最具行动价值**的错误分类（`Sensitive` 输入判定
///   优先于凭据级瞬态错误；否则取首个），驱动诚实提示文案分支。
///
/// 网络/凭据错误不向上抛——调用方计入 `dropped` 走诚实剥离，绝不中断主对话。
async fn ocr_image(
    data_base64: &str,
    media_type: &str,
    candidates: &[VisionCredential],
) -> Result<String, Option<LlmErrorKind>> {
    use base64::Engine as _;

    if candidates.is_empty() {
        return Err(None);
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64)
        .map_err(|e| {
            tracing::warn!(
                target: "ice_paw.modal",
                err = %e, "Image 块 base64 解码失败，跳过代读"
            );
            None // 解码失败 = 非调用失败
        })?;

    let mut last_kind: Option<LlmErrorKind> = None;
    for cred in candidates {
        match cred.describe(&bytes, media_type).await {
            Ok(text) => {
                tracing::info!(
                    target: "ice_paw.modal",
                    source = %cred.source,
                    chars = text.len(),
                    "视觉凭据代读图片成功"
                );
                return Ok(text);
            }
            Err(e) => {
                // 分类错误（敏感/限流/鉴权/网络...）驱动上层诚实提示文案——比旧实现的
                // 笼统「无凭据」准确（缺口③）。
                let msg = e.to_string();
                let kind = classify_llm_error(&msg);
                tracing::warn!(
                    target: "ice_paw.modal",
                    source = %cred.source,
                    err = %msg,
                    kind = ?kind,
                    "视觉凭据代读失败，尝试下一级"
                );
                // prefer：Sensitive（输入判定：图本身违规）优先于凭据级瞬态错误，
                // 否则取首个。旧实现 `= Some(kind)` 只留最后一个 → 把首选凭据正确给出
                // 的 Sensitive 丢成末位凭据的限流/余额，掩盖真正原因。
                last_kind = LlmErrorKind::prefer(last_kind, kind);
            }
        }
    }

    tracing::warn!(
        target: "ice_paw.modal",
        tried = ?candidates.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(),
        last_kind = ?last_kind,
        "所有视觉凭据代读均失败"
    );
    Err(last_kind)
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
        let out = adapt_blocks_for_vision(&blocks, true, &[], None).await;
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
        let out = adapt_blocks_for_vision(&blocks, false, &[], None).await;
        // 两张图都被剥离
        assert_eq!(out.dropped, 2);
        assert_eq!(out.ocr_replaced, 0);
        assert!(out.changed());
        // 文本块保留，无 Image 残留
        assert!(out.blocks.iter().all(|b| !b.is_image()));
        assert_eq!(out.blocks[0].as_text(), Some("看这张图"));
        // 末尾诚实提示含图片数 2 + 引导用户配置
        let hint = out.blocks.last().unwrap().as_text().unwrap();
        assert!(
            hint.contains("2 张图片"),
            "提示应含被剥离图片数，实际: {hint}"
        );
        assert!(hint.contains("视觉读取") || hint.contains("视觉模型"));
    }

    #[tokio::test]
    async fn non_vision_single_image_dropped_hint_count_is_one() {
        let blocks = vec![img("AAAA", "image/png")];
        let out = adapt_blocks_for_vision(&blocks, false, &[], None).await;
        assert_eq!(out.dropped, 1);
        let hint = out.blocks.last().unwrap().as_text().unwrap();
        assert!(
            hint.contains("1 张图片"),
            "单图剥离提示计数应为 1，实际: {hint}"
        );
    }

    // ---- 混合块：非图片块原样保留 ----

    #[tokio::test]
    async fn non_image_blocks_preserved_in_order() {
        let blocks = vec![
            ContentBlock::text("第一段"),
            img("AAAA", "image/png"),
            ContentBlock::text("第二段"),
        ];
        let out = adapt_blocks_for_vision(&blocks, false, &[], None).await;
        // 两个文本块顺序保留，中间图被剥，末尾加提示
        assert_eq!(out.blocks[0].as_text(), Some("第一段"));
        assert_eq!(out.blocks[1].as_text(), Some("第二段"));
        assert_eq!(out.dropped, 1);
        // 最后一块是诚实提示
        assert!(out
            .blocks
            .last()
            .unwrap()
            .as_text()
            .unwrap()
            .contains("1 张图片"));
    }

    // ---- 纯文本无图：非视觉也不注入多余提示 ----

    #[tokio::test]
    async fn non_vision_no_images_no_hint_injected() {
        let blocks = vec![ContentBlock::text("纯文本消息")];
        let out = adapt_blocks_for_vision(&blocks, false, &[], None).await;
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
        let out = adapt_blocks_for_vision(&blocks, false, &[], None).await;
        assert_eq!(out.dropped, 1);
        assert!(out
            .blocks
            .last()
            .unwrap()
            .as_text()
            .unwrap()
            .contains("1 张图片"));
    }

    // ---- dropped_hint：按失败原因分支（缺口③，纯函数单测）----

    #[test]
    fn dropped_hint_none_when_zero_dropped() {
        assert_eq!(dropped_hint(0, None), None);
        assert_eq!(dropped_hint(0, Some(LlmErrorKind::Sensitive)), None);
    }

    #[test]
    fn dropped_hint_no_credentials_guides_config() {
        // 无候选/解码失败 → 引导用户配置（旧实现唯一的文案，现在只是分支之一）
        let h = dropped_hint(2, None).expect("有提示");
        assert!(h.contains("2 张图片"));
        assert!(
            h.contains("未配置") && h.contains("视觉读取"),
            "无凭据应引导配置: {h}"
        );
    }

    #[test]
    fn dropped_hint_sensitive_gives_real_reason() {
        // 有凭据但敏感拒 → 说真实原因，而非笼统「无凭据」（缺口③核心）
        let h = dropped_hint(1, Some(LlmErrorKind::Sensitive)).expect("有提示");
        assert!(h.contains("1 张图片"));
        assert!(h.contains("安全审核"), "敏感拒应说原因: {h}");
        assert!(!h.contains("未配置"), "有凭据失败不应误导查配置: {h}");
    }

    #[test]
    fn dropped_hint_rate_limited_gives_reason() {
        let h = dropped_hint(1, Some(LlmErrorKind::RateLimited)).expect("有提示");
        assert!(h.contains("频繁"));
        assert!(!h.contains("未配置"));
    }

    #[test]
    fn dropped_hint_auth_gives_reason() {
        let h = dropped_hint(1, Some(LlmErrorKind::Auth)).expect("有提示");
        assert!(h.contains("密钥"));
    }

    #[test]
    fn dropped_hint_network_gives_reason() {
        let h = dropped_hint(1, Some(LlmErrorKind::Network)).expect("有提示");
        assert!(h.contains("网络") || h.contains("服务"));
    }

    #[test]
    fn dropped_hint_unknown_no_specific_reason() {
        // 有凭据但失败原因未识别 → 不伪造细节，诚实说「未能识别」
        let h = dropped_hint(1, Some(LlmErrorKind::Unknown)).expect("有提示");
        assert!(h.contains("未能识别"));
        assert!(
            !h.contains("未配置"),
            "Unknown 不应说「未配置」（其实有凭据）: {h}"
        );
    }

    // ---- strip_image_blocks_to_marker（历史路径，静默剥离）----

    #[test]
    fn strip_no_images_returns_unchanged() {
        let blocks = vec![
            ContentBlock::text("纯文本历史"),
            ContentBlock::text("第二条"),
        ];
        let out = strip_image_blocks_to_marker(&blocks);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|b| !b.is_image()));
    }

    #[test]
    fn strip_replaces_multiple_images_with_single_marker() {
        let blocks = vec![
            ContentBlock::text("看图"),
            img("AA", "image/png"),
            img("BB", "image/jpeg"),
            img("CC", "image/gif"),
        ];
        let out = strip_image_blocks_to_marker(&blocks);
        // 3 张图 → 1 条 marker（不重复噪声），文本块保留
        assert_eq!(out.len(), 2, "3 图应塌成 1 marker + 原文本块");
        assert!(out.iter().all(|b| !b.is_image()));
        // out = [原文本块 "看图", marker]（marker 在第一张图位置插入，后续图被吞）。
        // 直接取 out[1]，避免 find_map 命中第一个文本块 "看图"。
        let marker = out[1].as_text().unwrap();
        assert!(
            marker.contains("3 张图片"),
            "marker 应含图数，实际: {marker}"
        );
    }

    #[test]
    fn strip_marker_inserted_at_first_image_position() {
        let blocks = vec![
            ContentBlock::text("前言"),
            img("AA", "image/png"),
            ContentBlock::text("后语"),
        ];
        let out = strip_image_blocks_to_marker(&blocks);
        // 顺序：前言、marker、后语（marker 在首个图位）
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].as_text(), Some("前言"));
        assert_eq!(out[2].as_text(), Some("后语"));
        assert!(out[1].as_text().unwrap().contains("1 张图片"));
    }

    // ---- chat:processing 进度回调：覆盖三档行为 ----
    //
    // 心跳事件根因：60s 静默超时计时器假定「后端必有活动事件回报」，但 OCR 串行
    // 多图易超 60s。回调契约是 `每张图 OCR 完成（成功或失败）即触发一次`——
    // 让 ModalCapabilityStage 把回调内 emit 转成 chat:processing 心跳。
    // 三档测试覆盖「无图不触发」「视觉直通不触发」「构造 N 张图回调 N 次」。

    #[tokio::test]
    async fn progress_cb_not_called_when_no_images() {
        // 文本块 → 无回调调用（filter 出 0 张图）。这是高频路径（大多数消息走这条）。
        use std::sync::atomic::{AtomicU32, Ordering};
        let count = AtomicU32::new(0);
        let cb: ProgressCb<'_> = &|_done, _total| {
            count.fetch_add(1, Ordering::SeqCst);
        };
        let blocks = vec![ContentBlock::text("纯文本消息"), ContentBlock::text("第二条")];
        let _ = adapt_blocks_for_vision(&blocks, false, &[], Some(cb)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "无图不应触发回调");
    }

    #[tokio::test]
    async fn progress_cb_not_called_when_vision_passthrough() {
        // 视觉模型走早返回分支 → 不应触发回调（OCR 不跑）。
        use std::sync::atomic::{AtomicU32, Ordering};
        let count = AtomicU32::new(0);
        let cb: ProgressCb<'_> = &|_done, _total| {
            count.fetch_add(1, Ordering::SeqCst);
        };
        let blocks = vec![img("AA", "image/png"), img("BB", "image/jpeg")];
        let _ = adapt_blocks_for_vision(&blocks, true, &[], Some(cb)).await;
        assert_eq!(count.load(Ordering::SeqCst), 0, "视觉直通不应触发回调");
    }

    #[tokio::test]
    async fn progress_cb_total_reflects_image_count() {
        // total 字段必须 == blocks 中 Image 块数（不含 Text 块）。done 字段的
        // 1-based 序号在 OCR 串行调用中递增——这里只断言 total 的语义正确
        // （done 需真实 OCR 跑通才能精确断言，集成测试覆盖）。
        use std::sync::Mutex;
        let last_total = Mutex::new(None::<u32>);
        let cb: ProgressCb<'_> = &|_done, total| {
            *last_total.lock().unwrap() = Some(total);
        };
        // 无凭据（空数组）会触发 dropped 分支，但 done 仍然递增（失败也算进度）
        let blocks = vec![
            ContentBlock::text("前缀"),
            img("AA", "image/png"),
            ContentBlock::text("中插文本"), // 不计入 total
            img("BB", "image/jpeg"),
            img("CC", "image/gif"),
        ];
        let _ = adapt_blocks_for_vision(&blocks, false, &[], Some(cb)).await;
        let got = *last_total.lock().unwrap();
        assert_eq!(got, Some(3), "total 应为 Image 块数 3（Text 不计入）");
    }
}
