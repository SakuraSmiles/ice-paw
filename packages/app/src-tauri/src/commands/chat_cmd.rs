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
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::db::models::{HookConfig, HookPoint, NewMessage};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::infra::file_validation::validate_files;
use crate::infra::protocol::{
    AttachedFile, ChatMessage, ChatRoundStatePayload, ChatStartPayload, ConfigProposalResponse, ContentBlock,
    LlmProvider, ProposalDecision, SendMessageInput, ToolAuthResponse, validate_images,
};
use crate::commands::agent_cmd::AgentCmd;
use crate::harness::budget::LoopBudget;
use crate::harness::hooks::{has_actions, run_hooks};
use crate::harness::tool_executor::build_tool_ctx;
use crate::harness::chat_state::{CancellationToken, ChatState};
use crate::harness::loop_engine::{LoopConfig, LoopContext};
use crate::harness::observable::RoundState;
use crate::harness::provider;

use crate::harness::mcp::{McpServerManager, McpRegistry};
use crate::harness::authority::{PathAuthSession, PathWhitelistConfig};
use crate::context::pipeline::{AssembledContext, PipelineContext, PipelineRunner};

/// MemoryStage 折叠素材的 DB 加载上限（条数）。
///
/// Phase 2：解耦「DB 加载」与「发送上限」。加载充足历史给滚动摘要留折叠素材，
/// 而发送时由 MemoryStage（摘要压缩）+ TokenWindowStage（硬裁剪）分两级控制。
/// 部分加载（会话条数 > 此值）由 `covered_until_rowid` 按值定位安全兜底
/// （计数在部分加载下会静默丢消息，rowid 不会）。
const MEMORY_LOAD_LIMIT: i64 = 500;

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
fn materialize_file_blocks(
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
        let (kind, chunks) =
            crate::harness::doc::try_extract_chunks(&bytes, &ext)?.ok_or_else(|| {
                AppError::Validation(format!(
                    "附件 {} 的格式 .{} 不支持（允许：docx / xlsx / xls / pdf）",
                    f.name, ext
                ))
            })?;
        let kind_label = kind.label();
        let total_tokens: usize = chunks.iter().map(|c| estimate_tokens(&c.text)).sum();
        let bytes_len = bytes.len();
        tracing::info!(
            target: "ice_paw.attach",
            name = %f.name,
            decoded_bytes = bytes_len,
            ext = %ext,
            kind = %kind_label,
            chunks = chunks.len(),
            total_tokens,
            paginated = total_tokens > INLINE_BUDGET_TOKENS,
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

        // Phase B：文本提取为空（扫描件/纯图片/加密 PDF）→ 留存原始字节，供视觉工具
        // view_attachment_image 按需渲染页面。文本提取成功的附件不留存（agent 已有文本，
        // 省一个 10MB+ BLOB）。bytes 在此 move；bytes_len 已在上面捕获给 ExtractedFile。
        if total_tokens == 0 {
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
        });
    }

    // 全部文件处理完后的全局总页数（每个大文件提示里都用这个**最终**值，避免"截至本文件"
    // 的累计值在多文件场景下误导 LLM——此前 file A 的提示会把 total 写成它自己的页数）。
    let final_total_pages = global_idx as usize;

    // —— Pass 2：构建 blocks（UI 卡片 + 内联正文 / 大文件首页）——
    for e in &extracted {
        // 1. UI 卡片（不进 LLM；无论大小都 push 一张）
        blocks.push(ContentBlock::attachment(&e.name, &e.ext, e.bytes_len));

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
                     即可把指定页渲染成图片，由你的视觉能力直接读图（页号 1-based；返回的 JSON 摘要里含 \
                     total_pages，按需 page+1 继续翻页）。读取后再据实回答用户。\n\
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
            blocks.push(ContentBlock::text(format!(
                "<uploaded_file name=\"{}\" type=\"{}\">\n\
                 [系统提示：以下是系统自动从用户上传的附件提取的原文，非用户手打。请基于此内容回答用户。]\n\
                 {}\n\
                 </uploaded_file>",
                e.name, e.kind_label, body
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
            // ⚠️ String::truncate(new_len) 在 new_len 非 char 边界时 **panic**（不是回退！）。
            // INLINE_CHAR_CAP 是字节偏移，CJK/混合文本在 16000 字节处极易落在多字节字符中间。
            // 先回退到 ≤ cap 的最近 char 边界再 truncate。
            let mut cut = INLINE_CHAR_CAP;
            while cut > 0 && !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            body.push_str("\n\n[...内容过长已截断，完整内容请用 read_attachment_page 工具按页读取]");
        }

        blocks.push(ContentBlock::text(format!(
            "<uploaded_file name=\"{}\" type=\"{}\" total_pages=\"{}\">\n\
             [系统提示：以下是系统自动从用户上传的附件提取的原文，非用户手打。本附件已分页存储：\
             共 {} 页，对应全局页 {}~{}（本消息全部附件合计 {} 页）。此处仅展示前若干页。\
             如需后续内容，调用 read_attachment_page(message_id=\"{}\", page=<n>)\
             （1-based，全局 1~{}）读取指定页。]\n\
             {}\n\
             </uploaded_file>",
            e.name, e.kind_label, final_total_pages, file_pages, start_page, end_page,
            final_total_pages, message_id, final_total_pages, body
        )));
    }

    // 大文件块数据 + 视觉候选文件字节随返回值交调用方，**在 Pipeline 成功后**与消息行
    // 一起写库（见 send_message）。
    Ok((blocks, db_inputs, file_db_inputs))
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

    let conv_id = input.conversation_id.clone();
    let tools_enabled = input.tools_enabled;
    // P0-3: 会话级 model override（None = 使用 Agent 默认 model）
    let model_override = input.model.clone();
    // 用户原始文本 query（仅 Text 块，**不含**附件提取文本）——用于标题/钩子/检索，
    // 避免整份文档灌进 query 噪声。附件内容在下方 materialize 后进 final_blocks 给 LLM。
    let content_text = ContentBlock::join_text(&final_blocks);
    // M1.2: 提取当前用户消息的纯文本 query（仅 Text 块拼接；与 LLM 拼装时使用的 content_text 一致）
    let current_user_query = Some(content_text.clone());

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
    let (final_blocks, attach_db_inputs, attach_file_inputs) =
        match input.files.as_ref().filter(|v| !v.is_empty()) {
            Some(files) => {
                validate_files(files)?;
                materialize_file_blocks(&user_msg_id, final_blocks, files)?
            }
            None => (final_blocks, Vec::new(), Vec::new()),
        };

    // --- 3. 拼装上下文（A3-1：trait-based Pipeline） ---
    //
    // Phase 2：DB 加载固定 `MEMORY_LOAD_LIMIT`（500）条历史，不再耦合
    // `max_history_messages`。`max_history_messages` 现在是 MemoryStage 的
    // keep_n 地板（verbatim 保留窗），不再决定「加载/发送上限」。发送侧由
    // MemoryStage（摘要压缩）+ TokenWindowStage（token 硬裁剪）两级控制。
    //
    // M1.2: 同时查询「最近 10 次」tool 消息以填充 `tool_call_history`，
    // M1.4 后供 loop_engine 在每轮调用 list_tool_defs_with_query 打分时使用。
    let history =
        repo::message::list_by_conversation(pool.inner(), &conv_id, Some(MEMORY_LOAD_LIMIT), None).await?;
    // M1.2: 加载最近 10 条工具消息，提取 tool_use 块中的工具名
    let tool_call_history =
        repo::message::list_recent_tool_names(pool.inner(), &conv_id, 10).await?;

    // --- 3.5 注册 cancel token（必须先于 Pipeline，让 MemoryStage 摘要也能响应取消）---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(target: "ice_paw.chat", "send_message: 会话 {} 已有在途生成任务", conv_id);
    })?;

    // RAII 守卫：到此 token 已注册，若下方任意 `?` 早返回（Pipeline/DB 写/emit/spawn 前失败），
    // 守卫 drop 时自动 unregister，避免 conv_id 永久残留 ChatState → is_streaming 永不翻转 →
    // 后续 send_message 全部命中「已有在途任务」、会话卡死需重启 App。
    // spawn 成功后在函数末尾 disarm，把注销责任移交给 stream_loop 的 finalize_*。
    // 用 clone 让守卫独立持有 conv_id，本体的 conv_id 仍可 move 进 spawn_stream_loop。
    let conv_id_guard = conv_id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // 显式走 PipelineRunner：构造 PipelineContext + 注册 8 个 Stage：
    // Template → OsContext → SystemPrompt → History → ToolFailureFold → Memory
    // → TokenWindow → Final（M1.4 起不再含 ToolTrimStage，工具裁剪下沉到 loop_engine）。
    // 后续新增 Stage 在 default_pipeline 注册即可，无需改动业务编排层。
    //
    // M1.5: PipelineContext 现在携带 conversation_id + cancel_token，
    //       MemoryStage 需要二者才能调 summary_provider 并响应取消。
    let mut pipeline_ctx = PipelineContext::new(
        pool.inner().clone(),
        agent.clone(),
        None,
        history,
        final_blocks,
        tools_enabled,
        current_user_query.clone(),
        tool_call_history.clone(),
        // Phase 0: 用 agent 的 context_window 构造上下文预算。
        // 解析优先级：agent 显式覆盖 → (provider, model) 已知模型默认 → 128K 兜底。
        // max_input_tokens 现被 Phase 1（TokenWindowStage 硬裁）+ Phase 2
        // （MemoryStage 折叠触发/目标比例 fold_trigger_tokens / fold_target_tokens）消费。
        {
            let max_input = agent
                .context_window
                .map(|v| v as usize)
                .or_else(|| {
                    crate::harness::provider::default_context_window(&agent.provider, &agent.model)
                })
                .unwrap_or(128_000);
            crate::context::token::ContextBudget {
                max_input_tokens: max_input,
                ..crate::context::token::ContextBudget::default()
            }
        },
        conv_id.clone(),
        cancel_token.clone(),
    );

    // 项目 workspace + 上下文目录注入 Pipeline
    if let Some(ref pid) = conv.project_id {
        if let Ok(proj) = repo::project::get_by_id(pool.inner(), pid).await {
            pipeline_ctx.project_workspace = proj.workspace_path;
        }
        // 项目上下文目录：{default_ws}/projects/{project_id}/
        if let Ok(prefs) = repo::preferences::get_all(pool.inner()).await {
            if let Some(ref ws) = prefs.default_workspace_path {
                let dir = format!(
                    "{}/projects/{}",
                    ws.trim_end_matches(['/', '\\']),
                    pid
                );
                pipeline_ctx.project_context_dir = Some(dir);
            }
        }
    }

    // M1.5: 构造 LlmSummaryProvider 注入 Pipeline
    use crate::harness::summary_provider::LlmSummaryProvider;
    use crate::context::memory::SummaryProvider;
    let summary_provider: Box<dyn SummaryProvider> = Box::new(
        LlmSummaryProvider::new(llm_provider.clone(), api_key.clone()),
    );
    PipelineRunner::default_pipeline(pool.inner(), Some(summary_provider))
        .run(&mut pipeline_ctx)
        .await?;

    // M1.5: emit chat:summary-injected if MemoryStage triggered.
    // 注：前端目前**未** listen 此事件（types/index.ts 有 payload 类型，但无 listen 调用）。
    // emit 保留（3 行、无害、面向未来）；折叠每数轮一次，逐次 toast 噪声大，
    // 摘要注入的前端可见性（toast / 上下文检视器）留待未来 UX 决策。
    if let Some(event) = pipeline_ctx.summary_event {
        let _ = app.emit("chat:summary-injected", event);
    }

    let mut assembled = AssembledContext {
        messages: pipeline_ctx.messages,
        user_blocks: pipeline_ctx.user_blocks,
    };

    // --- 4. Pipeline 成功 → 落库（用户消息 + 分页块 + assistant 占位） ---
    // 全部 DB 写入推迟到此刻：前置任一失败（conv/agent/provider/materialize/Pipeline）
    // 都不落任何行，无孤儿。用户消息 content = content_text（仅手打文本，不含附件提取正文）。
    repo::message::create(
        pool.inner(), &user_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(), role: "user".into(),
            content: content_text, token_count: None, error: None,
            model: None,
        },
    ).await?;
    // 回填 content_blocks（含 materialize 产出的 Attachment 卡片 + 提取正文 Text 块）。
    let blocks_json = serde_json::to_string(&assembled.user_blocks).unwrap_or_else(|_| "[]".into());
    repo::message::update_content_blocks(pool.inner(), &user_msg_id, &blocks_json).await?;
    // 大附件分页块：消息行已存在（FK 父满足）。幂等：先清旧再批量插。
    if !attach_db_inputs.is_empty() {
        repo::message_attachment::delete_by_message(pool.inner(), &user_msg_id).await?;
        repo::message_attachment::insert_batch(pool.inner(), &user_msg_id, &attach_db_inputs).await?;
    }
    // Phase B 视觉候选文件字节（扫描件等，文本提取为空）：同批写入，CASCADE 跟消息生命周期。
    if !attach_file_inputs.is_empty() {
        repo::message_attachment_file::delete_by_message(pool.inner(), &user_msg_id).await?;
        repo::message_attachment_file::insert_batch(pool.inner(), &user_msg_id, &attach_file_inputs)
            .await?;
    }

    let asst_msg_id = Uuid::new_v4().to_string();
    // P0-3: 助手消息的 model 字段使用 override（若有），否则回退 Agent 默认 model。
    // 这样 messages 表能正确记录每次回复实际使用的模型。
    let effective_model = model_override.clone().unwrap_or_else(|| agent.model.clone());
    repo::message::create(
        pool.inner(), &asst_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(), role: "assistant".into(),
            content: String::new(), token_count: None, error: None,
            model: Some(effective_model.clone()),
        },
    ).await?;

    // --- 5. emit chat:start（cancel_token 已在 3.5 注册） ---
    // 含附件时把后端 materialize 后的 content_blocks 带给前端，patch 乐观用户消息
    // （否则前端拿不到提取正文，附件详情弹窗全显示「无提取文本」）。
    let has_files = input.files.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
    app.emit("chat:start", ChatStartPayload {
        conversation_id: conv_id.clone(),
        user_message_id: user_msg_id.clone(),
        assistant_message_id: asst_msg_id.clone(),
        user_content_blocks: if has_files { Some(blocks_json.clone()) } else { None },
    })?;

    // --- 6. spawn 流式协程 ---
    // 从 Agent extra_params 读取工具调用最大轮数 + Token 预算
    let tool_max_rounds = agent.tool_max_rounds();
    let agent_max_tokens = agent.max_total_tokens();
    // model-aware 预算兜底：agent 未显式设 max_total_tokens 时，按上下文窗口自适应
    //（默认 3× 窗口）。累计成本语义下 Σ(prompt_i+completion_i) = provider 真实毛成本，
    // prompt 随历史单调增长；3× 窗口让正常长会话不被误杀，失控循环仍兜得住。
    // 窗口解析与 Phase 0 上下文预算同源（agent 显式 → 模型表 → 128K 兜底）。
    let budget_max_tokens = agent_max_tokens.unwrap_or_else(|| {
        let window = agent
            .context_window
            .map(|v| v as usize)
            .or_else(|| {
                crate::harness::provider::default_context_window(&agent.provider, &agent.model)
            })
            .unwrap_or(128_000);
        window.saturating_mul(3)
    });
    // 使用共享的工具授权注册表（与 lib.rs install_listener 实例一致）
    let shared_auth_registry = (*auth_registry).clone();

    // 单轮输出上限：模式 E 治本——agent.max_tokens 与模型策展表取 max（只抬不降）。
    // 把过低的默认/历史值（4096/16384）抬到模型真实输出能力，减少 finish_reason=length
    // 触发的自动续写次数（续写在 loop_engine 内处理）。yaml 显式调高者被尊重；
    // 未知模型回退 16384（DB 默认）。
    let effective_max_tokens = agent.max_tokens.max(
        crate::harness::provider::default_max_output_tokens(&agent.provider, &agent.model)
            .unwrap_or(16_384) as i32,
    );

    // 组装对话 tool_registry：直接从 global registry 快照（boot 时已启动全部 server）。
    // per_agent server 的 workspace 后台异步绑定，不阻塞消息发送。
    let tool_registry = if tools_enabled {
        // 默认全开：所有已注册工具（内置 + 全局/per_agent MCP server，含平台元工具）
        // 对每个 agent 可用。enabled_tools 白名单是排他快照——一旦设定 agent 即被锁死，
        // 后续新增的 MCP server（如 GLM）不会自动可用。前端 agent 工具配置 UI 尚未实现，
        // 故统一全开；待 UI 落地后再启用精细化白名单（届时恢复 register_names_from 分支）。
        let reg = McpRegistry::from_map(global_registry.snapshot().await);

        // 后台异步绑定 per_agent server workspace（不阻塞消息发送）
        if let Some(workspace) = agent.workspace_path.as_deref() {
            let mcp_configs = repo::mcp_server::list_all(pool.inner()).await.unwrap_or_else(|e| {
                tracing::warn!(target: "ice_paw.chat", "加载 MCP server 配置失败: {e}");
                Vec::new()
            });
            let mgr = Arc::clone(&mcp_manager);
            let reg = Arc::clone(&global_registry);
            let ws = workspace.to_string();
            tokio::spawn(async move {
                for cfg in mcp_configs.iter().filter(|c| c.enabled && c.scope == "per_agent") {
                    mgr.rebind_workspace_if_needed(&cfg.id, &ws, &reg).await;
                }
            });
        }
        reg
    } else {
        McpRegistry::new()
    };

    // --- 6.5 钩子：ConversationStart（对话开始，inject_prompt 追加到 system_prompt）---
    // 在 tool_registry 组装完成后、spawn 前执行：此时 system 消息已由 Pipeline 拼装进
    // assembled.messages。注入结果随 messages 进入流式循环，对本轮所有工具子轮持续生效
    //（system 消息本就每轮重建，非 DB 行；此处注入不写库，每次 send_message 重新注入）。
    if has_actions(&hooks, HookPoint::ConversationStart) {
        let hook_ctx = build_tool_ctx(
            pool.inner(),
            conv_id.clone(),
            conv.agent_id.clone(),
            conv.project_id.clone(),
            Some(api_key.clone()),
        )
        .await;
        match run_hooks(HookPoint::ConversationStart, &hooks, &hook_ctx, &tool_registry).await {
            Ok(outcome) => {
                if let Some(inj) = outcome.injected_prompt {
                    inject_into_system(&mut assembled.messages, &inj);
                }
            }
            Err(e) => tracing::warn!(
                target: "ice_paw.hooks",
                "ConversationStart 钩子执行失败（忽略）: {}", e
            ),
        }
    }

    spawn_stream_loop(
        app, pool.inner().clone(), llm_provider, api_key,
        assembled.messages, agent.temperature, effective_max_tokens,
        cancel_token, conv_id, user_msg_id, asst_msg_id, tools_enabled,
        current_user_query, tool_call_history,
        model_override, Some(effective_model), tool_max_rounds, budget_max_tokens, shared_auth_registry,
        tool_registry,
        conv.agent_id.clone(), conv.project_id.clone(),
        hooks,
    );
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

/// 把钩子注入的 prompt 追加到 system 消息（ConversationStart 用）。
///
/// - 若已存在 system 消息：追加一个 Text 块（provider 适配层会把同一条消息内的
///   多个 Text 块拼接，等价于追加到 system_prompt）。
/// - 若无 system 消息：新建一条置于首位。
fn inject_into_system(messages: &mut Vec<ChatMessage>, injected: &str) {
    if let Some(sys_msg) = messages.iter_mut().find(|m| m.role == "system") {
        sys_msg.content.push(ContentBlock::text(injected.to_string()));
    } else {
        messages.insert(0, ChatMessage::from_text("system", injected.to_string()));
    }
}

/// spawn LLM 流式协程，把编排结果交给 `harness::loop_engine::stream_loop`。
///
/// W6.2: 入参已封装到 [`LoopContext`] 后传给 `stream_loop`。
///
/// M1.4: 移除 `trimmed_tool_defs` 参数——Pipeline 不再做工具裁剪，
/// loop_engine 在每轮独立调 `list_tool_defs_with_query()` 打分。
///
/// P0-3: 新增 `model_override` 参数（会话级 model 覆盖，None = 用 Agent 默认），
/// 透传到 LoopContext.model，由 stream_loop 传给 provider.stream_chat()。
///
/// 注意：`spawn_stream_loop` 自身的形参列表仍是 12 个（编排层分散收集
/// 各 Tauri State/输入），所以 `#[allow(clippy::too_many_arguments)]`
/// 仍需保留；要消除这个 lint 需要更彻底地把编排也下沉到 `LoopContext`
/// 子构造器（不在 W6.2 范围内）。
#[allow(clippy::too_many_arguments)]
fn spawn_stream_loop(
    app: AppHandle, pool: SqlitePool, provider: Arc<dyn LlmProvider>,
    api_key: String, messages: Vec<ChatMessage>, temperature: f64, max_tokens: i32,
    cancel_token: CancellationToken, conv_id: String, user_msg_id: String,
    asst_msg_id: String, tools_enabled: bool,
    query: Option<String>, call_history: Vec<String>,
    model_override: Option<String>,
    asst_model: Option<String>,
    tool_max_rounds: Option<u32>,
    budget_max_tokens: usize,
    auth_registry: crate::harness::tool_executor::ToolAuthRegistry,
    tool_registry: McpRegistry,
    agent_id: String,
    project_id: Option<String>,
    hooks: HookConfig,
) {
    tokio::spawn(async move {
        // ★ RAII Drop 守卫：无论此任务如何退出（正常完成 / panic / runtime 关闭时被 drop），
        // 都保证注销 ChatState 中的 cancel_token。这消除了 scopeguard disarm（L303）后
        // 唯一清理路径失效的风险——之前若 stream_loop panic 或 runtime 关闭时 future 被
        // drop 而未执行到 finalize_* → cleanup → unregister，token 永久残留导致会话卡死。
        let _cleanup_guard = scopeguard::guard((), {
            let app = app.clone();
            let conv_id = conv_id.clone();
            move |_| {
                let chat_state = app.state::<ChatState>();
                chat_state.unregister(&conv_id);
            }
        });

        // tool_registry 由 send_message 组装（global server + per-agent server），直接使用
        // W2.4: maintain observable state across the stream loop
        let mut observable = RoundState::default();
        // W4.1: 传入 LoopBudget（优先使用 Agent 配置）
        let budget = LoopBudget {
            max_tool_rounds: tool_max_rounds.unwrap_or(LoopBudget::default().max_tool_rounds),
            max_total_tokens: budget_max_tokens,
            ..LoopBudget::default()
        };
        let emit_app = app.clone();

        // A2-3: 使用共享的工具授权注册表（与 lib.rs install_listener 同一个实例）
        // 这样前端 chat:tool-auth-response 事件能匹配到正确的 oneshot sender。
        let auth_registry = auth_registry;
        // A2-3: 本次会话级已授权路径表
        let auth_session = PathAuthSession::new();
        // A2-3: 路径白名单配置（当前为空 → 全部走 Confirm 流程）
        let whitelist = PathWhitelistConfig::default();

        // A6: LoopConfig(不可变配置) + LoopContext(配置 + 可变消息缓冲)
        let config = LoopConfig {
            conv_id: conv_id.clone(),
            asst_msg_id,
            user_msg_id,
            agent_id,
            project_id,
            app,
            pool,
            provider,
            api_key,
            temperature,
            max_tokens,
            tool_registry,
            tools_enabled,
            auth_registry,
            auth_session,
            whitelist,
            cancel: cancel_token,
            budget,
            query,
            call_history,
            model: model_override,
            asst_model,
            hooks,
        };
        let mut ctx = LoopContext::new(config, messages);
        crate::harness::loop_engine::stream_loop(&mut ctx, &mut observable).await;
        // W2.4: emit final round-state after stream_loop completes
        let _ = emit_app.emit(
            "chat:round-state",
            ChatRoundStatePayload {
                conversation_id: conv_id,
                round: observable.round,
                elapsed_ms: observable.elapsed_ms,
                tokens_prompt: observable.tokens_prompt,
                tokens_completion: observable.tokens_completion,
                cached_tokens: observable.cached_tokens,
                retry_count: observable.retry_count,
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_into_system_appends_block_to_existing_system() {
        let mut messages = vec![
            ChatMessage::from_text("system", "You are X."),
            ChatMessage::from_text("user", "hi"),
        ];
        inject_into_system(&mut messages, "ALWAYS use JSON.");
        // 仍是同一条 system 消息（追加块，非新增消息）
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content.len(), 2);
        let joined = messages[0].content_text();
        assert!(joined.contains("You are X."));
        assert!(joined.contains("ALWAYS use JSON."));
    }

    #[test]
    fn inject_into_system_creates_system_when_absent() {
        let mut messages = vec![ChatMessage::from_text("user", "hi")];
        inject_into_system(&mut messages, "system rule");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content_text(), "system rule");
        // 原 user 消息被推到第二位
        assert_eq!(messages[1].role, "user");
    }
}
