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
            if t.is_empty() {
                None
            } else {
                Some(t.to_owned())
            }
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
        let result = validate_send_input(Some("from content".into()), Some(blocks)).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].as_text(), Some("from blocks"));
    }

    #[test]
    fn validate_empty_blocks_falls_back_to_content() {
        // 空 content_blocks 被过滤 → 回退到 content
        let result = validate_send_input(Some("fallback".into()), Some(vec![])).unwrap();
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
}
