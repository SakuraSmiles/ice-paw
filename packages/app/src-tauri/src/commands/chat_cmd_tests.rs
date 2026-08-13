//! chat_cmd 单元测试
//!
//! 测试 send_message / stop_generation 的关键路径。
//! 完整集成测试（含 Pipeline+stream_loop）放在 tests/ 目录。

#[cfg(test)]
mod tests {
    use crate::commands::agent_cmd::{AgentCmd, MockAgentCmd};
    use crate::db::models::AgentRow;
    use crate::harness::chat_state::{CancellationToken, ChatState};
    // =========================================================================
    // stop_generation 测试
    // =========================================================================

    #[test]
    fn stop_nonexistent_conversation_does_not_error() {
        let cs = ChatState::default();
        // stop 不存在的会话 → 仅 warn，不报错
        assert!(!cs.stop("nonexistent"));
    }

    #[test]
    fn stop_active_conversation_cancels_token() {
        let cs = ChatState::default();
        let conv_id = "conv-stop-1";
        let token = cs.start(conv_id).unwrap();
        assert!(!token.is_cancelled());

        let stopped = cs.stop(conv_id);
        assert!(stopped);
        assert!(token.is_cancelled());
    }

    #[test]
    fn start_duplicate_conversation_returns_err() {
        let cs = ChatState::default();
        let conv_id = "conv-dup-1";
        let _token = cs.start(conv_id).unwrap();
        // 同一会话重复 start 应报错
        assert!(cs.start(conv_id).is_err());
    }

    #[test]
    fn unregister_removes_token() {
        let cs = ChatState::default();
        let conv_id = "conv-unreg-1";
        let token = cs.start(conv_id).unwrap();
        assert!(!token.is_cancelled());

        cs.unregister(conv_id);
        // unregister 后可以重新 start（不再报"已有在途任务"）
        let token2 = cs.start(conv_id).unwrap();
        assert!(!token2.is_cancelled());
        // 旧 token 仍然独立存在（但已被移除，不再影响 ChatState）
        drop(token);
        drop(token2);
    }

    // =========================================================================
    // 输入校验逻辑测试（纯函数，无需 Tauri State）
    // =========================================================================

    /// 复制自 send_message 的校验逻辑——作为纯函数便于测试
    fn validate_send_input(
        content: Option<String>,
        content_blocks: Option<Vec<crate::infra::protocol::ContentBlock>>,
    ) -> Result<Vec<crate::infra::protocol::ContentBlock>, String> {
        let blocks = content_blocks.filter(|v| !v.is_empty());
        let legacy = content.as_ref().and_then(|s| {
            let t = s.trim();
            if t.is_empty() { None } else { Some(t.to_owned()) }
        });
        match (blocks, legacy) {
            (Some(b), _) => Ok(b),
            (None, Some(t)) => Ok(vec![crate::infra::protocol::ContentBlock::text(t)]),
            (None, None) => Err("content 或 content_blocks 至少提供一个".into()),
        }
    }

    #[test]
    fn validate_empty_input_returns_err() {
        let result = validate_send_input(None, None);
        assert!(result.is_err());
    }

    #[test]
    fn validate_whitespace_only_content_returns_err() {
        let result = validate_send_input(Some("   ".into()), None);
        assert!(result.is_err());
    }

    #[test]
    fn validate_legacy_content_fallback() {
        let result = validate_send_input(Some("hello".into()), None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_text(), Some("hello"));
    }

    #[test]
    fn validate_content_blocks_priority() {
        use crate::infra::protocol::ContentBlock;
        let blocks = vec![ContentBlock::text("from blocks")];
        // content_blocks 优先于 legacy content
        let result = validate_send_input(
            Some("from content".into()),
            Some(blocks),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_text(), Some("from blocks"));
    }

    #[test]
    fn validate_empty_blocks_falls_back_to_content() {
        // 空 content_blocks 被过滤 → 回退到 content
        let result = validate_send_input(
            Some("fallback".into()),
            Some(vec![]),
        )
        .unwrap();
        assert_eq!(result[0].as_text(), Some("fallback"));
    }

    // =========================================================================
    // MockAgentCmd 集成测试（验证 trait object 通路）
    // =========================================================================

    fn make_test_agent_row(id: &str, name: &str) -> AgentRow {
        AgentRow {
            id: id.into(),
            name: name.into(),
            provider: "openai".into(),
            model: "gpt-4".into(),
            system_prompt: String::new(),
            api_key_ref: id.into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 4096,
            extra_params: "{}".into(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages: None,
            tool_trim_threshold: None,
            context_window: None,
            enabled_tools: None,
            supports_vision: 0,
            description: String::new(),
            avatar: None,
            workspace_path: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn mock_agent_cmd_get_with_credentials() {
        let mock = MockAgentCmd::new();
        let row = make_test_agent_row("agent-1", "Test Agent");
        mock.seed(row, "sk-test-key".to_string(), None);

        let result: crate::error::AppResult<_> = mock.get_with_credentials("agent-1").await;
        let awc = result.unwrap();
        assert_eq!(awc.agent.id, "agent-1");
        assert_eq!(awc.agent.name, "Test Agent");
        assert_eq!(awc.api_key, "sk-test-key");
    }

    #[tokio::test]
    async fn mock_agent_cmd_not_found() {
        let mock = MockAgentCmd::new();
        let result: crate::error::AppResult<_> = mock.get_with_credentials("nonexistent").await;
        assert!(result.is_err());
    }

    // =========================================================================
    // CancelToken 生命周期
    // =========================================================================

    #[test]
    fn cancel_token_default_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancel_token_cancel_sets_flag() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_token_clone_shares_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
    }

    // =========================================================================
    // build_modality_hint 测试（事1：视觉模态元信息注入）
    // =========================================================================

    #[test]
    fn modality_hint_counts_images() {
        use crate::infra::protocol::ContentBlock;
        let blocks = vec![
            ContentBlock::Image { data: "x".into(), media_type: "image/png".into() },
            ContentBlock::text("hi"),
            ContentBlock::Image { data: "y".into(), media_type: "image/jpeg".into() },
        ];
        let hint = crate::commands::chat_cmd::build_modality_hint(&blocks).expect("有图应返回提示块");
        let s = ContentBlock::join_text(std::slice::from_ref(&hint));
        assert!(s.contains("2 张图片"), "应含图片数 2，实际: {s}");
        assert!(s.contains("无需调用"), "应告知无需调图片工具，实际: {s}");
    }

    #[test]
    fn modality_hint_none_without_image() {
        use crate::infra::protocol::ContentBlock;
        let blocks = vec![ContentBlock::text("纯文本消息")];
        assert!(
            crate::commands::chat_cmd::build_modality_hint(&blocks).is_none(),
            "无图不应注入提示块"
        );
    }

    // =========================================================================
    // materialize_file_blocks 软失败测试（0 字节 / 损坏附件不阻塞整条消息）
    // =========================================================================

    /// 0 字节 PDF：base64 解码为空 → try_extract_chunks Err → 软失败为诚实提示，
    /// 返回 Ok（不阻塞），原始字节不留存（渲染必失败），无分页块。
    #[test]
    fn materialize_soft_fails_on_empty_pdf() {
        use crate::commands::chat_cmd::materialize_file_blocks;
        use crate::infra::protocol::AttachedFile;

        let files = vec![AttachedFile {
            name: "empty.pdf".into(),
            data: String::new(), // 0 字节（空 base64 → 解码为空 Vec）
        }];
        let (blocks, db_chunks, db_files) =
            materialize_file_blocks("msg-empty", vec![], &files)
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
        use crate::commands::chat_cmd::materialize_file_blocks;
        use crate::infra::protocol::AttachedFile;

        let files = vec![AttachedFile {
            name: "blank.docx".into(),
            data: String::new(),
        }];
        let (blocks, _db_chunks, db_files) =
            materialize_file_blocks("msg-blank", vec![], &files)
                .expect("0 字节 docx 应软失败而非 Err");
        assert!(db_files.is_empty(), "0 字节 docx 不留存原始字节");
        let texts: Vec<&str> = blocks.iter().filter_map(|b| b.as_text()).collect();
        assert!(
            texts.iter().any(|t| t.contains("extracted=\"failed\"")),
            "docx 空文件也应注入失败提示，实际: {texts:?}"
        );
    }

    // =====================================================================
    // PDF 视觉字节留存 + 提示（层①②治本，2026-08-13：混合型 PDF 不再丢字节）
    // 真实触发面：用户传一份图纸 PDF（282KB 只提取到 359 字标签），旧门槛 total_tokens==0
    // 判其"提取成功"而丢字节 → agent 永久丧失视觉、转去翻文件系统。治本后所有非损坏 PDF
    // 都留字节 + 注入提示，agent 可按需调 view_attachment_image 渲染整页读图。
    // =====================================================================

    #[test]
    fn pdf_vision_bytes_stored_for_every_non_failed_pdf() {
        use crate::commands::chat_cmd::should_store_pdf_vision_bytes as gate;
        use crate::infra::file_validation::MAX_FILE_SIZE;

        // 混合型 PDF（图纸，有零星文字）：治本核心——旧门槛会漏，现在必留
        assert!(gate("pdf", 282_000, false), "混合型 PDF 应留字节（治本核心）");
        // 纯文字 PDF：也留（由 agent 自行决定是否用视觉，不替它预测）
        assert!(gate("pdf", 1_000, false));
        // 达上传上限的 PDF 仍留（不设更小二级门槛，避免大扫描件回退到 bug）
        assert!(gate("pdf", MAX_FILE_SIZE, false), "达上传上限仍留字节");
        // 超上传上限：理论上送不到（validate_files 已拦），防御性 false
        assert!(!gate("pdf", MAX_FILE_SIZE + 1, false));
        // 损坏 / 0 字节：渲染必失败，不留（白占 BLOB）
        assert!(!gate("pdf", 100, true), "extract_failed 不留字节");
        // Office 文档：当前无渲染路径，不留
        assert!(!gate("docx", 5_000, false), "docx 无渲染路径，不留");
        assert!(!gate("xlsx", 5_000, false));
    }

    #[test]
    fn pdf_vision_hint_guides_agent_to_render() {
        use crate::commands::chat_cmd::pdf_vision_hint;
        let h = pdf_vision_hint("msg-abc");
        // 指引工具 + 带 message_id（工具必填参数）+ page 示例
        assert!(h.contains("view_attachment_image"), "应指引工具: {h}");
        assert!(h.contains(r#"message_id="msg-abc""#), "应带 message_id: {h}");
        assert!(h.contains("page=1"), "应给 page 示例: {h}");
        // 覆盖混合型场景关键词（让 agent 识别"文字不完整"）
        assert!(h.contains("图纸"), "应覆盖图纸类混合型场景: {h}");
    }
}
