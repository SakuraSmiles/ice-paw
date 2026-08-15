//! Chat Tauri Commands 入口 — 仅编排，不含业务逻辑。
//!
//! - `send_message`：入参校验 → 取 agent/api_key → 拼装上下文 → 写库占位 → spawn stream_loop
//! - `stop_generation`：触发 ChatState 上的 CancellationToken
//!
//! 业务分布：protocol → infra::protocol | 上下文 → context::pipeline | 调度 → harness::loop_engine
//!           错误 → harness::error_mapping | 收尾 → harness::cleanup

use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::infra::file_validation::validate_files;
use crate::infra::protocol::{
    AttachedFile, ConfigProposalResponse, ContentBlock,
    ProposalDecision, SendMessageInput, ToolAuthResponse, strip_empty_image_blocks, validate_images,
};
use crate::commands::agent_cmd::AgentCmd;
use crate::harness::chat_state::ChatState;
use crate::harness::provider;
use crate::harness::session_runner;

use crate::harness::mcp::{McpServerManager, McpRegistry};

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
/// **纯函数（无 IO / 无 DB）**：只做 base64 解码 + 提取 + 拼装 blocks，并把大文件的分页
/// 块数据收集到返回值的 `db_inputs`。**调用方负责在 Pipeline 成功后**把消息行与块行一起
/// 写库——这样 Pipeline / conv / agent 任一前置失败都不落任何 DB 行，**无孤儿用户消息**
/// （分页块外键父 = 用户消息，必须消息行先存在；故二者都在 Pipeline 后写入）。
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
            e.name, e.kind_label, final_total_pages, file_pages, start_page, end_page,
            final_total_pages, message_id, final_total_pages, body, vision_hint
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
pub(crate) fn should_store_pdf_vision_bytes(ext: &str, bytes_len: usize, extract_failed: bool) -> bool {
    ext == "pdf"
        && !extract_failed
        && bytes_len <= crate::infra::file_validation::MAX_FILE_SIZE
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
/// 仅图片触发：文档已被 `materialize_file_blocks` 转成 `<uploaded_file>` 文本块，
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

/// 发送消息 — 触发 LLM 流式生成。
///
/// REQ-XC-010: 依赖 `Arc<dyn AgentCmd>` trait object，而非具体
/// `SqlAgentCmd` 类型。这样可以在测试中注入 `MockAgentCmd`。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_message(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    chat_state: State<'_, ChatState>,
    agent_cmd: State<'_, Arc<dyn AgentCmd>>,
    input: SendMessageInput,
    auth_registry: State<'_, crate::harness::tool_executor::ToolAuthRegistry>,
    global_registry: State<'_, Arc<McpRegistry>>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    route_registry: State<'_, crate::harness::read_route::ReadRouteRegistry>,
) -> AppResult<()> {
    tracing::info!(target: "ice_paw.chat", "send_message 被调用: conv={} model={:?} tools={}",
        input.conversation_id, input.model, input.tools_enabled);
    // --- 1. 入参校验：content_blocks 优先，回退到 legacy content ---
    let final_blocks = {
        let blocks = input.content_blocks.clone().filter(|v| !v.is_empty());
        let legacy = input.content.as_ref().and_then(|s| {
            let t = s.trim();
            if t.is_empty() { None } else { Some(t.to_owned()) }
        });
        match (blocks, legacy) {
            (Some(b), _) => b,
            (None, Some(t)) => vec![ContentBlock::text(t)],
            // 纯附件无文本：允许（materialize 会填充 Attachment + 提取正文）
            (None, None) if input.files.as_ref().map(|v| !v.is_empty()).unwrap_or(false) => Vec::new(),
            (None, None) => return Err(AppError::Validation(
                "content 或 content_blocks 至少提供一个".into(),
            )),
        }
    };
    validate_images(&final_blocks)?;
    // 0 字节图片能通过 validate_images（尺寸 0 ≤ 上限），但发给 LLM 会被 400 拒绝。
    // 软剥离：空图片块移除 + 注入诚实提示，绝不阻塞整条消息（与文档附件软失败同策略）。
    let final_blocks = strip_empty_image_blocks(final_blocks);

    let conv_id = input.conversation_id.clone();
    let tools_enabled = input.tools_enabled;
    // P0-3: 会话级 model override（None = 使用 Agent 默认 model）
    let model_override = input.model.clone();
    // 用户原始文本 query（仅 Text 块，**不含**附件提取文本）——用于标题/钩子/检索，
    // 避免整份文档灌进 query 噪声。附件内容在下方 materialize 后进 final_blocks 给 LLM。
    // （runner 内部以 content_text 同时作消息正文与检索 query。）
    let content_text = ContentBlock::join_text(&final_blocks);

    // --- 2. 取会话 + agent + api_key → 创建 provider ---
    // （先于写消息：conv/agent/provider 任一失败都应**不落任何 DB 行**，避免孤儿用户消息。）
    let conv = repo::conversation::get_by_id(pool.inner(), &conv_id).await?;
    let agent_with_creds = agent_cmd.get_with_credentials(&conv.agent_id).await?;
    let agent = agent_with_creds.agent;
    let api_key = agent_with_creds.api_key;
    let base_url = agent_with_creds.base_url.as_deref();
    // 钩子配置（来自 agent.yaml `hooks`；纯文件，不进 DB）
    let hooks = agent_with_creds.hooks;

    let llm_provider = provider::create_provider(
        &agent.provider, &agent.model, base_url, agent.cache_prompt != 0,
    )?;

    // Phase 3 + Phase A：office/pdf 附件 → 后端提取为 Text 块追加到 final_blocks 末尾。
    // 文件是输入模态（LLM 读不了 base64 二进制），物化为 Text：不进 ContentBlock 枚举、
    // base64 不落盘 DB、provider 适配层零改动。提取失败 → 整条消息拒绝（fail-fast，先于 Pipeline）。
    //
    // materialize 是**纯函数**（只解码+提取+拼 blocks+收集分页块数据），**不写 DB**。
    // 用户消息行与分页块行都推迟到 Pipeline 成功后一起写——这样 conv/agent/provider/
    // Pipeline/materialize 任一失败都不落任何 DB 行，**无孤儿用户消息**（分页块外键父
    // = 用户消息，二者同批写入，消息行先于块行）。
    //
    // user_msg_id 预生成（仅 UUID 字符串，无 IO）：materialize 需把它嵌进大文件首页的
    // read_attachment_page 工具提示里；真正落库时复用同一 id。
    let user_msg_id = Uuid::new_v4().to_string();
    let (mut final_blocks, attach_db_inputs, attach_file_inputs) =
        match input.files.as_ref().filter(|v| !v.is_empty()) {
            Some(files) => {
                validate_files(files)?;
                materialize_file_blocks(&user_msg_id, final_blocks, files)?
            }
            None => (final_blocks, Vec::new(), Vec::new()),
        };

    // 事1 + 事2：视觉模态元信息注入。仅当 agent「有效支持视觉」时插入"你已直接收到 N 张图、
    // 无需调图片工具"的元提示——纠正视觉 agent「看到了却说没看到」的认知偏差。
    // 非视觉 agent 不插此提示（其图片由 ModalCapabilityStage 走代读/诚实剥离，提示由该 Stage 负责）。
    // 持久化用的原始 blocks：用户真实发送内容（含原图 + 附件卡片/正文）。
    // ⚠️ 视觉适配（ModalCapabilityStage 对非视觉 agent 把 Image 代读/剥离）只改发给 LLM 的
    // 视图（pipeline_ctx.final_blocks → user_blocks），**绝不能污染落库的用户消息**——否则
    // 非视觉 agent 的历史回看会丢图（用户气泡从 content_blocks 取 Image 块渲染；bug 表现：
    // 发送时看得到图、agent 回答后从 DB 刷新就消失）。故在此（materialize 后、build_modality_hint
    // 元提示前）clone 一份原始 blocks 专供落库，与发给 LLM 的适配视图彻底解耦。
    let persist_blocks: Vec<ContentBlock> = final_blocks.clone();

    let eff_vision = crate::harness::provider::effective_supports_vision(
        agent.supports_vision,
        &agent.provider,
        &agent.model,
    );
    if eff_vision {
        if let Some(hint) = build_modality_hint(&final_blocks) {
            final_blocks.insert(0, hint);
        }
    }

    // --- 3. 注册 cancel token（必须先于 Pipeline，让 MemoryStage 摘要也能响应取消）---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(target: "ice_paw.chat", "send_message: 会话 {} 已有在途生成任务", conv_id);
    })?;

    // RAII 守卫：到此 token 已注册，若下方任意 `?` 早返回（Pipeline/DB 写/emit/spawn 前失败），
    // 守卫 drop 时自动 unregister，避免 conv_id 永久残留 ChatState → is_streaming 永不翻转 →
    // 后续 send_message 全部命中「已有在途任务」、会话卡死需重启 App。
    // spawn 成功后在函数末尾 disarm，把注销责任移交给 stream_loop 的 finalize_*。
    let conv_id_guard = conv_id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 4. 委派 session_runner：历史解析 → Pipeline → 落库 → spawn 流式循环 ---
    // MA-1 抽取：send_message 保留输入预处理（校验/附件物化/视觉提示），「一次完整
    // agent 回合」的编排内核复用 session_runner::run_agent_turn（agent 委派走同一
    // 内核——专家用自己的 provider/key 跑完整循环）。用户路径 fire-and-forget：
    // 完成信号 Receiver 直接 drop，前端靠流式事件感知进度。
    let _turn_done = session_runner::run_agent_turn(
        &session_runner::TurnEnv {
            app: app.clone(),
            pool: pool.inner().clone(),
            route_registry: route_registry.inner(),
            global_registry: Arc::clone(global_registry.inner()),
            mcp_manager: Arc::clone(mcp_manager.inner()),
            auth_registry: auth_registry.inner().clone(),
        },
        session_runner::AgentTurnInput {
            conv,
            agent,
            hooks,
            provider: llm_provider,
            api_key,
            user_msg_id,
            content_text,
            llm_blocks: final_blocks,
            persist_blocks,
            attach_db_inputs,
            attach_file_inputs,
            emit_user_blocks: input.files.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
            tools_enabled,
            model_override,
            cancel_token,
        },
    )
    .await?;
    // spawn 成功：注销责任已移交 stream_loop（其 finalize_success/finalize_cancel → cleanup →
    // unregister），解除守卫，避免此处 Ok 返回时误注销导致 is_streaming 提前翻转。
    scopeguard::ScopeGuard::into_inner(cancel_guard);
    Ok(())
}

/// 停止指定会话的流式生成。
#[tauri::command]
pub async fn stop_generation(
    chat_state: State<'_, ChatState>,
    conversation_id: String,
) -> AppResult<()> {
    if !chat_state.stop(&conversation_id) {
        tracing::warn!(target: "ice_paw.chat",
            "stop_generation: 会话 {} 无在途生成任务", conversation_id);
    }
    Ok(())
}

// ============================================================================
// 前端 → 后端 响应通道（invoke 命令）
//
// 设计说明（重要）：
// 原先审批/授权响应用 chat:config-proposal-response / chat:tool-auth-response
// 事件（前端 emit → 后端 app.listen）。但 Tauri v2 中前端 emit 是 webview 作用域、
// 后端 listen 是全局监听器——作用域不匹配导致事件被静默丢弃，双通道从未工作，
// 表现为「点批准后 agent 仍等满 120s 超时」。
// 改为 invoke 命令（Tauri 推荐的前端→后端请求-响应 IPC），可靠送达。
// ============================================================================

/// 前端审批结果 → 唤醒后端 propose_config_change 的 oneshot 等待者。
///
/// 扁平入参（与前端 `ConfigProposalResponse` 类型一致：decision 为字符串），
/// 命令内转 [`ProposalDecision`] enum 再交由 registry。
#[tauri::command]
pub async fn respond_config_proposal(
    proposal_registry: State<'_, crate::harness::proposal_registry::ProposalRegistry>,
    input: ConfigProposalResponseInput,
) -> AppResult<()> {
    let decision = match input.decision.as_str() {
        "approved" => ProposalDecision::Approved,
        "modified" => ProposalDecision::Modified {
            changes: input.changes.unwrap_or_default(),
        },
        "rejected" => ProposalDecision::Rejected { reason: input.reason },
        other => {
            return Err(AppError::Validation(format!(
                "未知提案 decision: '{other}'（需 approved/modified/rejected）"
            )))
        }
    };
    let handled = proposal_registry
        .respond(ConfigProposalResponse {
            request_id: input.request_id,
            decision,
        })
        .await;
    if !handled {
        tracing::warn!(
            target: "ice_paw.mgmt",
            "提案 respond：未找到匹配的 request_id（可能已超时/取消）"
        );
    }
    Ok(())
}

/// 前端工具授权结果 → 唤醒后端 wait_for_auth_response 的 oneshot 等待者。
#[tauri::command]
pub async fn respond_tool_auth(
    auth_registry: State<'_, crate::harness::tool_executor::ToolAuthRegistry>,
    input: ToolAuthResponse,
) -> AppResult<()> {
    let handled = auth_registry.respond(input).await;
    if !handled {
        tracing::warn!(
            target: "ice_paw.tool_auth",
            "授权 respond：未找到匹配的 request_id（可能已超时/取消）"
        );
    }
    Ok(())
}

/// 前端审批响应的扁平入参（decision 为字符串，匹配前端类型）。
/// 命令内转为 [`ProposalDecision`] enum 再交由 registry。
#[derive(serde::Deserialize)]
pub struct ConfigProposalResponseInput {
    pub request_id: String,
    /// `"approved"` | `"modified"` | `"rejected"`
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub changes: Option<HashMap<String, String>>,
}

