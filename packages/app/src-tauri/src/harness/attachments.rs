//! 附件物化 — office/pdf 附件 → ContentBlock 的输入预处理（S3 从 chat_cmd.rs 迁出）。
//!
//! 纯函数集合（无 IO / 无 DB）：只做 base64 解码 + 提取 + 拼装 blocks，并把大文件的
//! 分页块数据收集到返回值。**调用方（chat_cmd::send_message）负责在 Pipeline 成功后**
//! 把消息行与块行一起写库——这样 Pipeline / conv / agent 任一前置失败都不落任何
//! DB 行，**无孤儿用户消息**（分页块外键父 = 用户消息，必须消息行先存在）。
//!
//! 与相邻模块的分工：扩展名识别/容器解析 → [`crate::harness::doc`]；上传上限/白名单
//! 校验 → [`crate::infra::file_validation`]（在物化**之前**由 send_message 调用）；
//! 历史图片的视觉适配 → [`crate::harness::modal`]（本模块只管**当前输入**）。

use crate::error::{AppError, AppResult};
use crate::infra::protocol::{AttachedFile, ContentBlock};

/// 小文件阈值：提取正文总 token ≤ 此值则**全量内联**（零行为变化，不分页）。
/// 超过则按块分页，只内联首页 + `read_attachment_page` 工具按页取。
const INLINE_BUDGET_TOKENS: usize = 4_000;

/// 内联首页正文的硬字符上限（~4000 token 的混合 CJK/ASCII 上限）。
/// 累加切块逻辑按 token 预算停，但单个超大块（如巨表 sheet）仍可能超——此 cap 兜底截断。
const INLINE_CHAR_CAP: usize = 16_000;

/// 把 office/pdf 附件物化为「Attachment 卡片 + Text 提取正文」并追加到 `blocks` 末尾。
///
/// 每个附件：base64 解码 → [`crate::harness::doc::try_extract_chunks`]（按文件名扩展名
/// 分发，返回分块）→ 追加：
/// 1. [`ContentBlock::Attachment`] —— 纯 UI 元信息（name/kind/size），让用户气泡与
///    历史记录渲染出"上传了 xxx.docx"卡片；provider 适配层显式跳过，**不发给 LLM**。
/// 2. [`ContentBlock::Text`] —— `<uploaded_file>` 包裹的提取正文，发给 LLM 阅读。
///    用户气泡只渲染 `msg.content` + 图片 + Attachment 卡片，不渲染 content_blocks 的
///    Text，故这坨提取正文对用户不可见（仅 LLM 读）。
///
/// **分页（Phase A 治本 PDF >1M 读不到）**：单个附件提取总 token > [`INLINE_BUDGET_TOKENS`]
/// 时不再整篇灌进一个 Text block（会撑爆 LLM 窗口 / 被裁 / 被拒）。改为：
/// - 各块文本写入 `message_attachments` 表（`message_id` 外键 CASCADE，跟消息生命周期），
///   `idx` **跨文件全局连续递增**（表不区分文件，靠注入头里的页码范围告知 LLM）。
/// - 只把该附件前若干块（≤预算）内联进 Text block，并附 `read_attachment_page` 工具提示。
///
/// 小附件（≤预算）仍全量内联，**零行为变化**。
///
/// **后端为附件元信息唯一来源**：先剥离入参里前端可能传入的 Attachment 块（乐观显示
/// 用），再从 `files` 重建。提取失败返回 [`Err`]，整条 `send_message` 拒绝（让用户知道
/// 哪个附件读不了），绝不静默吞。
///
/// `message_id`：用户消息 ID（预生成的 UUID 字符串，仅用于注入提示里的工具参数；
/// 真正的 DB 写入由调用方在消息落库时用同一 id）。
pub(crate) fn materialize_file_blocks(
    message_id: &str,
    mut blocks: Vec<ContentBlock>,
    files: &[AttachedFile],
) -> AppResult<(
    Vec<ContentBlock>,
    Vec<crate::db::repo::message_attachment::AttachmentChunkInput>,
    Vec<crate::db::repo::message_attachment_file::AttachmentFileInput>,
)> {
    use base64::Engine as _;
    use std::path::Path;

    use crate::context::token::estimate_tokens;
    use crate::db::repo::message_attachment::AttachmentChunkInput;
    use crate::db::repo::message_attachment_file::AttachmentFileInput;

    // 后端为唯一真源：丢弃前端乐观传入的 Attachment 块，下面从 files 重建
    blocks.retain(|b| !matches!(b, ContentBlock::Attachment { .. }));

    // 单个附件的提取结果（两遍处理：先全部提取定全局页码，再拼 blocks）。
    struct ExtractedFile {
        name: String,
        ext: String,
        bytes_len: usize,
        kind_label: &'static str,
        chunks: Vec<crate::harness::doc::TextChunk>,
        total_tokens: usize,
        /// 大文件在全局页序列里的 1-based 区间；小文件为 None。
        page_range: Option<(usize, usize)>,
        /// 提取失败原因（空文件 / 损坏容器）。Some → 软失败，Pass 2 注入诚实提示，不阻塞整条消息。
        extract_failed: Option<String>,
        /// 该 PDF 的原始字节是否已留存供 view_attachment_image 按需渲染（层①治本）。
        /// Pass 2 在分支③④据此决定是否拼接 [`pdf_vision_hint`]。
        vision_available: bool,
    }

    // —— Pass 1：解码 + 提取 + 收集 db_inputs + 给大文件分配全局页码区间 ——
    // 全局 idx 跨文件连续递增（message_attachments 不区分文件，靠注入提示告知 LLM 各文件范围）。
    let mut extracted: Vec<ExtractedFile> = Vec::new();
    let mut db_inputs: Vec<AttachmentChunkInput> = Vec::new();
    // Phase B：视觉候选（文本提取为空）的原始字节，供 view_attachment_image 渲染。
    let mut file_db_inputs: Vec<AttachmentFileInput> = Vec::new();
    let mut global_idx: i64 = 0;

    for (file_idx, f) in files.iter().enumerate() {
        tracing::info!(
            target: "ice_paw.attach",
            name = %f.name,
            b64_len = f.data.len(),
            "收到附件，开始处理"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&f.data)
            .map_err(|e| AppError::Validation(format!("附件 {} base64 解码失败: {e}", f.name)))?;
        let ext = Path::new(&f.name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let started = std::time::Instant::now();
        let bytes_len = bytes.len();
        // 提取（软失败：空文件 / 损坏容器不阻塞整条消息，降级为诚实提示）。
        // try_extract_chunks：Ok(Some)=成功；Ok(None)=扩展名未识别（validate_files 已白名单
        // 拦截，理论不可达，兜底当损坏）；Err=文件空 / 损坏（非法 ZIP / PDF 头）。
        // 改造前这里直接 `?` 上抛 → 单个坏附件让整条 send_message 失败（用户痛点）。
        let (kind_label, chunks, extract_failed): (
            &'static str,
            Vec<crate::harness::doc::TextChunk>,
            Option<String>,
        ) = match crate::harness::doc::try_extract_chunks(&bytes, &ext) {
            Ok(Some((kind, chunks))) => (kind.label(), chunks, None),
            Ok(None) => (
                "unknown",
                Vec::new(),
                Some(format!("格式 .{ext} 未被提取器识别")),
            ),
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.attach",
                    name = %f.name, ext = %ext, bytes = bytes_len, err = %e,
                    "附件提取失败（空文件/损坏），软失败为诚实提示（不阻塞整条消息）"
                );
                let reason = if bytes_len == 0 {
                    "文件为空（0 字节）".to_string()
                } else {
                    format!("文件可能损坏或格式不合规（{bytes_len} 字节）：{e}")
                };
                ("unknown", Vec::new(), Some(reason))
            }
        };
        let total_tokens: usize = chunks.iter().map(|c| estimate_tokens(&c.text)).sum();
        tracing::info!(
            target: "ice_paw.attach",
            name = %f.name,
            decoded_bytes = bytes_len,
            ext = %ext,
            kind = %kind_label,
            chunks = chunks.len(),
            total_tokens,
            paginated = total_tokens > INLINE_BUDGET_TOKENS,
            failed = extract_failed.is_some(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "附件提取完成"
        );

        // 大文件：登记分页块（全局 idx 连续）+ 记录该文件的页码区间。
        // 小文件：不分页（page_range = None，Pass 2 全量内联）。
        let page_range = if total_tokens > INLINE_BUDGET_TOKENS {
            let start = (global_idx + 1) as usize;
            for c in &chunks {
                db_inputs.push(AttachmentChunkInput {
                    idx: global_idx,
                    name: f.name.clone(),
                    kind: kind_label.to_string(),
                    label: c.label.clone(),
                    text: c.text.clone(),
                    token_est: estimate_tokens(&c.text) as i64,
                });
                global_idx += 1;
            }
            let end = global_idx as usize;
            Some((start, end))
        } else {
            None
        };

        // 层①（治本，2026-08-13）：留存 PDF 原始字节供 view_attachment_image 按需渲染。
        // 旧门槛 `total_tokens == 0` 是一刀切悬崖——混合型 PDF（图纸 / 图表 / 扫描带 OCR）
        // 有零星可提取文字但实质内容是图形，被判"提取成功"而丢字节 → 永久丧失视觉
        // （实测一份 282KB 户型图只提取到 359 字标签，墙体/布局全丢，agent 转去翻文件系统）。
        // 改为：所有非损坏、未超上传上限的 PDF 都留存字节，由能读到上下文的 agent 自行判断
        // 是否调 view_attachment_image（层②在注入提示里告知）。非 PDF（Office）无渲染路径，不存。
        // ⚠️ extract_failed（0 字节 / 损坏容器）渲染必失败、白占 BLOB → 排除。
        let vision_available =
            should_store_pdf_vision_bytes(&ext, bytes_len, extract_failed.is_some());
        if vision_available {
            file_db_inputs.push(AttachmentFileInput {
                idx: file_idx as i64,
                name: f.name.clone(),
                ext: ext.clone(),
                bytes,
            });
        }

        extracted.push(ExtractedFile {
            name: f.name.clone(),
            ext,
            bytes_len,
            kind_label,
            chunks,
            total_tokens,
            page_range,
            extract_failed,
            vision_available,
        });
    }

    // 全部文件处理完后的全局总页数（每个大文件提示里都用这个**最终**值，避免"截至本文件"
    // 的累计值在多文件场景下误导 LLM——此前 file A 的提示会把 total 写成它自己的页数）。
    let final_total_pages = global_idx as usize;

    // —— Pass 2：构建 blocks（UI 卡片 + 内联正文 / 大文件首页）——
    for e in &extracted {
        // 1. UI 卡片（不进 LLM；无论大小都 push 一张）
        blocks.push(ContentBlock::attachment(&e.name, &e.ext, e.bytes_len));

        // —— 提取失败（空文件 / 损坏容器）诚实提示 ——
        // 与下面的「空提取（扫描件）」分支区别：这里是文件本身有问题（0 字节 / 非法 ZIP·PDF 头），
        // 原始字节**未留存**（渲染也必失败），故绝不指引 view_attachment_image；如实告知 + 建议重传。
        if let Some(reason) = &e.extract_failed {
            blocks.push(ContentBlock::text(format!(
                "<uploaded_file name=\"{}\" type=\"{}\" extracted=\"failed\">\n\
                 [系统提示：附件 {name} 无法读取——{reason}。文件已成功接收，但无法解析出任何内容，\
                 原始字节未留存（渲染同样会失败，故无需调 view_attachment_image）。\
                 请如实告诉用户：该附件当前无法读取，建议检查文件是否损坏或为空后重新上传。]\n\
                 </uploaded_file>",
                e.name,
                e.kind_label,
                name = e.name,
                reason = reason,
            )));
            continue;
        }

        // —— 空提取守卫（Phase B 落地：扫描件/图片型 PDF 现可视觉读取）——
        // 文本层抽不到内容：扫描件 / 纯图片 PDF、只含内嵌图片的 docx、极稀疏 xlsx、加密 PDF。
        // 绝不能伪造"以下是提取的原文"空壳——那会让 LLM 误以为文件在磁盘上、继而用文件工具全盘翻找
        // （实测一次会话 agent 白跑了十几个 run_command/list_directory）。
        // 改发如实提示：明说提取为空 + 该文件不在磁盘/工作目录 + 指示可走的路径：
        //   - PDF：原始字节已留存 → 指引调用 view_attachment_image 渲染成图、视觉读图（Phase B）
        //   - 非 PDF：当前无视觉回退 → 如实转告用户、建议手动复制段落
        if e.total_tokens == 0 {
            tracing::warn!(
                target: "ice_paw.attach",
                name = %e.name,
                kind = %e.kind_label,
                ext = %e.ext,
                bytes = e.bytes_len,
                "附件文本提取为空（疑似扫描件/纯图片/加密 PDF）"
            );
            let hint = if e.ext == "pdf" {
                format!(
                    "已尝试自动提取该附件的文本，但结果为空——通常是扫描件、纯图片型或加密 PDF。\
                     文件已成功接收（{bytes} 字节），但**它不在磁盘或当前工作目录中**，请勿尝试用文件工具\
                     （run_command / list_directory / read_file 等）去翻找它。\n\
                     **该 PDF 的原始字节已留存：调用 view_attachment_image(message_id=\"{mid}\", page=1)** \
                     即可把指定页渲染成图片并读出其内容（若当前模型不支持视觉，系统会自动用全局视觉\
                     配置把该页代读成文本返回；页号 1-based，返回的 JSON 摘要里含 total_pages，\
                     按需 page+1 继续翻页）。读取后再据实回答用户。\n\
                     若调用失败（例如加密 PDF 无法渲染），请如实告诉用户该附件当前无法读取，\
                     建议其手动复制相关段落贴入对话。",
                    bytes = e.bytes_len,
                    mid = message_id
                )
            } else {
                format!(
                    "已尝试自动提取该附件的文本，但结果为空。文件已成功接收（{bytes} 字节），\
                     但**它不在磁盘或当前工作目录中**，请勿尝试用文件工具去翻找它。\
                     当前版本对该格式（.{ext}）的空提取暂无视觉回退路径——请如实告诉用户该附件暂时无法读取，\
                     建议其手动复制相关段落贴入对话。",
                    bytes = e.bytes_len,
                    ext = e.ext
                )
            };
            blocks.push(ContentBlock::text(format!(
                "<uploaded_file name=\"{}\" type=\"{}\" extracted=\"empty\">\n\
                 [系统提示：{hint}]\n\
                 </uploaded_file>",
                e.name, e.kind_label
            )));
            continue;
        }

        if e.total_tokens <= INLINE_BUDGET_TOKENS {
            // 小文件：全量内联（零行为变化）。多块拼接（docx 段 / xlsx sheet）。
            let body = e
                .chunks
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let vision_hint = if e.vision_available {
                pdf_vision_hint(message_id)
            } else {
                String::new()
            };
            blocks.push(ContentBlock::text(format!(
                "<uploaded_file name=\"{}\" type=\"{}\">\n\
                 [系统提示：以下是系统自动从用户上传的附件提取的原文，非用户手打。请基于此内容回答用户。]\n\
                 {}{}\n\
                 </uploaded_file>",
                e.name, e.kind_label, body, vision_hint
            )));
            continue;
        }

        // —— 大文件：首页内联 + 工具提示 ——
        let (start_page, end_page) = e.page_range.expect("大文件必有 page_range");
        let file_pages = end_page - start_page + 1;

        // 内联首页：从前若干块累加到 INLINE_BUDGET_TOKENS（至少内联第一块，
        // 即便它本身超预算——再由 INLINE_CHAR_CAP 字符硬截兜底）。
        let mut body = String::new();
        let mut acc = 0usize;
        for c in &e.chunks {
            let t = estimate_tokens(&c.text);
            if !body.is_empty() && acc + t > INLINE_BUDGET_TOKENS {
                break;
            }
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&format!("--- {} ---\n{}", c.label, c.text));
            acc += t;
        }
        if body.len() > INLINE_CHAR_CAP {
            // 字节截断走统一的安全函数（回退到 char 边界，永不 panic）。
            body = crate::infra::strings::truncate_to_byte_boundary(
                &body,
                INLINE_CHAR_CAP,
                Some("\n\n[...内容过长已截断，完整内容请用 read_attachment_page 工具按页读取]"),
            );
        }

        let vision_hint = if e.vision_available {
            pdf_vision_hint(message_id)
        } else {
            String::new()
        };
        blocks.push(ContentBlock::text(format!(
            "<uploaded_file name=\"{}\" type=\"{}\" total_pages=\"{}\">\n\
             [系统提示：以下是系统自动从用户上传的附件提取的原文，非用户手打。本附件已分页存储：\
             共 {} 页，对应全局页 {}~{}（本消息全部附件合计 {} 页）。此处仅展示前若干页。\
             如需后续内容，调用 read_attachment_page(message_id=\"{}\", page=<n>)\
             （1-based，全局 1~{}）读取指定页。]\n\
             {}{}\n\
             </uploaded_file>",
            e.name,
            e.kind_label,
            final_total_pages,
            file_pages,
            start_page,
            end_page,
            final_total_pages,
            message_id,
            final_total_pages,
            body,
            vision_hint
        )));
    }

    // 大文件块数据 + 视觉候选文件字节随返回值交调用方，**在 Pipeline 成功后**与消息行
    // 一起写库（见 send_message）。
    Ok((blocks, db_inputs, file_db_inputs))
}

/// 层①：是否为 `view_attachment_image` 留存 PDF 原始字节。纯函数，便于单测。
///
/// 旧门槛 `total_tokens == 0` 让混合型 PDF（图纸/图表等"有零星文字但实质是图形"）丢字节、
/// 永久丧失视觉。改为所有非损坏、未超 [`crate::infra::file_validation::MAX_FILE_SIZE`]（上传
/// 校验已拦，此处复用为上限）的 PDF 都留存，由 agent 按层②提示自行决定是否调视觉工具。
/// Office 文档无渲染路径，不存（省 BLOB）。`ext` 须为小写（调用方 [`materialize_file_blocks`]
/// 已用 `to_ascii_lowercase` 归一）。
pub(crate) fn should_store_pdf_vision_bytes(
    ext: &str,
    bytes_len: usize,
    extract_failed: bool,
) -> bool {
    ext == "pdf" && !extract_failed && bytes_len <= crate::infra::file_validation::MAX_FILE_SIZE
}

/// 层②：非空提取 PDF 的视觉读取提示。拼进 `<uploaded_file>` 内联正文末尾，告知 agent 在
/// 提取文字不完整（图纸/图表/扫描件/含图形信息）时可调 `view_attachment_image` 渲染整页读图。
/// 仅当字节已留存（[`should_store_pdf_vision_bytes`] 为真）时由 [`materialize_file_blocks`] 拼接。
pub(crate) fn pdf_vision_hint(message_id: &str) -> String {
    format!(
        "\n[补充：本附件为 PDF，原始字节已留存。若上面提取的文字对回答用户问题不完整\
         （例如这是图纸 / 图表 / 扫描件 / 含图形信息的文档，文字层无法表达布局、图形、\
         尺寸等视觉内容），可调用 view_attachment_image(message_id=\"{mid}\", page=1) \
         把该页渲染成图、用视觉读取（page 为 1-based；返回的 JSON 摘要含 total_pages，\
         按需 page+1 翻页；当前模型无视觉能力时，系统会用全局视觉配置把该页代读成文本返回）。]",
        mid = message_id
    )
}

/// 构造"视觉模态元信息"提示块（事1，2026-08-12）。
///
/// agent 通过 Image 块（视觉通道）已直接看到图片，但工具列表里的视觉 MCP
/// （`analyze_image` 等）常让它误以为"图片要走工具才算数"，于是声称"没拿到图"并
/// 误导用户重新上传。本函数在 blocks 含图片时返回一个 Text 提示块，显式告知 agent
/// 已直接收到 N 张图、无需再调图片分析工具（仅在 OCR / 图像对比 / UI 转代码等深度
/// 处理时才调相应工具）。无图返回 None。纯函数，便于单测。
///
/// 仅图片触发：文档已被 [`materialize_file_blocks`] 转成 `<uploaded_file>` 文本块，
/// agent 直接读到提取的正文，无此认知问题。提示块与 `<uploaded_file>` 同模式落库
/// （进用户消息 content_blocks，历史回看亦可知当时附图数）。
pub(crate) fn build_modality_hint(blocks: &[ContentBlock]) -> Option<ContentBlock> {
    let image_count = blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image { .. }))
        .count();
    if image_count == 0 {
        return None;
    }
    Some(ContentBlock::text(format!(
        "[系统提示：你已通过视觉通道直接收到本次消息附带的 {image_count} 张图片，\
         无需调用任何图片分析工具即可看到其完整内容；仅在需要 OCR / 图像对比 / UI 转代码\
         等深度处理时才调用相应工具。]"
    )))
}

// =========================================================================
// 单元测试（S3 从 chat_cmd_tests.rs 随迁——测的就是本模块的函数）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- build_modality_hint（事1：视觉模态元信息注入）---

    #[test]
    fn modality_hint_counts_images() {
        let blocks = vec![
            ContentBlock::Image {
                data: "x".into(),
                media_type: "image/png".into(),
            },
            ContentBlock::text("hi"),
            ContentBlock::Image {
                data: "y".into(),
                media_type: "image/jpeg".into(),
            },
        ];
        let hint = build_modality_hint(&blocks).expect("有图应返回提示块");
        let s = ContentBlock::join_text(std::slice::from_ref(&hint));
        assert!(s.contains("2 张图片"), "应含图片数 2，实际: {s}");
        assert!(s.contains("无需调用"), "应告知无需调图片工具，实际: {s}");
    }

    #[test]
    fn modality_hint_none_without_image() {
        let blocks = vec![ContentBlock::text("纯文本消息")];
        assert!(build_modality_hint(&blocks).is_none(), "无图不应注入提示块");
    }

    // --- materialize_file_blocks 软失败（0 字节 / 损坏附件不阻塞整条消息）---

    /// 0 字节 PDF：base64 解码为空 → try_extract_chunks Err → 软失败为诚实提示，
    /// 返回 Ok（不阻塞），原始字节不留存（渲染必失败），无分页块。
    #[test]
    fn materialize_soft_fails_on_empty_pdf() {
        let files = vec![AttachedFile {
            name: "empty.pdf".into(),
            data: String::new(), // 0 字节（空 base64 → 解码为空 Vec）
        }];
        let (blocks, db_chunks, db_files) = materialize_file_blocks("msg-empty", vec![], &files)
            .expect("0 字节附件应软失败为诚实提示，而非 Err 阻塞整条消息");

        // 0 字节 / extract_failed → 不留存原始字节（渲染同样失败，白占 BLOB）
        assert!(db_files.is_empty(), "0 字节附件不应留存原始字节");
        assert!(db_chunks.is_empty(), "0 字节附件无分页块");

        // 应注入 extracted="failed" 的诚实提示，并说明 0 字节
        let texts: Vec<&str> = blocks.iter().filter_map(|b| b.as_text()).collect();
        assert!(
            texts.iter().any(|t| t.contains("extracted=\"failed\"")),
            "应注入 extracted=failed 提示，实际: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("0 字节")),
            "提示应说明 0 字节，实际: {texts:?}"
        );
    }

    /// 0 字节 docx（非法 ZIP 容器）同样软失败，验证非 PDF 格式也覆盖。
    #[test]
    fn materialize_soft_fails_on_empty_docx() {
        let files = vec![AttachedFile {
            name: "blank.docx".into(),
            data: String::new(),
        }];
        let (blocks, _db_chunks, db_files) = materialize_file_blocks("msg-blank", vec![], &files)
            .expect("0 字节 docx 应软失败而非 Err");
        assert!(db_files.is_empty(), "0 字节 docx 不留存原始字节");
        let texts: Vec<&str> = blocks.iter().filter_map(|b| b.as_text()).collect();
        assert!(
            texts.iter().any(|t| t.contains("extracted=\"failed\"")),
            "docx 空文件也应注入失败提示，实际: {texts:?}"
        );
    }

    // --- PDF 视觉字节留存 + 提示（层①②治本，2026-08-13：混合型 PDF 不再丢字节）---
    // 真实触发面：用户传一份图纸 PDF（282KB 只提取到 359 字标签），旧门槛 total_tokens==0
    // 判其"提取成功"而丢字节 → agent 永久丧失视觉、转去翻文件系统。治本后所有非损坏 PDF
    // 都留字节 + 注入提示，agent 可按需调 view_attachment_image 渲染整页读图。

    #[test]
    fn pdf_vision_bytes_stored_for_every_non_failed_pdf() {
        use crate::infra::file_validation::MAX_FILE_SIZE;

        // 混合型 PDF（图纸，有零星文字）：治本核心——旧门槛会漏，现在必留
        assert!(
            should_store_pdf_vision_bytes("pdf", 282_000, false),
            "混合型 PDF 应留字节（治本核心）"
        );
        // 纯文字 PDF：也留（由 agent 自行决定是否用视觉，不替它预测）
        assert!(should_store_pdf_vision_bytes("pdf", 1_000, false));
        // 达上传上限的 PDF 仍留（不设更小二级门槛，避免大扫描件回退到 bug）
        assert!(
            should_store_pdf_vision_bytes("pdf", MAX_FILE_SIZE, false),
            "达上传上限仍留字节"
        );
        // 超上传上限：理论上送不到（validate_files 已拦），防御性 false
        assert!(!should_store_pdf_vision_bytes(
            "pdf",
            MAX_FILE_SIZE + 1,
            false
        ));
        // 损坏 / 0 字节：渲染必失败，不留（白占 BLOB）
        assert!(
            !should_store_pdf_vision_bytes("pdf", 100, true),
            "extract_failed 不留字节"
        );
        // Office 文档：当前无渲染路径，不留
        assert!(
            !should_store_pdf_vision_bytes("docx", 5_000, false),
            "docx 无渲染路径，不留"
        );
        assert!(!should_store_pdf_vision_bytes("xlsx", 5_000, false));
    }

    #[test]
    fn pdf_vision_hint_guides_agent_to_render() {
        let h = pdf_vision_hint("msg-abc");
        // 指引工具 + 带 message_id（工具必填参数）+ page 示例
        assert!(h.contains("view_attachment_image"), "应指引工具: {h}");
        assert!(
            h.contains(r#"message_id="msg-abc""#),
            "应带 message_id: {h}"
        );
        assert!(h.contains("page=1"), "应给 page 示例: {h}");
        // 覆盖混合型场景关键词（让 agent 识别"文字不完整"）
        assert!(h.contains("图纸"), "应覆盖图纸类混合型场景: {h}");
    }
}
