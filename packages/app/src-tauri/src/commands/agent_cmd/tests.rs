//! agent_cmd 单元测试（`#[cfg(test)]`，从原 `agent_cmd.rs` God module 拆出，U3-5 ③）。
//!
//! 覆盖三块：校验/端点跟随纯函数、MockAgentCmd trait 行为、出生 yaml 模板去雷。

use super::*;
use super::default_yaml::build_default_agent_yaml_content;
use super::validation::{
    default_url_on_provider_switch, manual_materializable, resolve_base_url_arg,
    validate_new_agent, validate_update_model_fields_conflict,
};
use crate::db::models::{AgentRow, NewAgent};
use crate::error::AppError;

fn sample_agent_row(id: &str, name: &str) -> AgentRow {
    AgentRow {
        id: id.to_string(),
        name: name.to_string(),
        provider: "anthropic".to_string(),
        model: "claude-3-5-sonnet".to_string(),
        system_prompt: "you are a helpful assistant".to_string(),
        api_key_ref: id.to_string(),
        base_url: None,
        temperature: 0.7,
        max_tokens: 1024,
        extra_params: "{}".to_string(),
        sort_order: 0,
        cache_prompt: 0,
        max_history_messages: None,
        context_window: None,
        enabled_tools: None,
        tool_scopes: None,
        supports_vision: 0,
        description: String::new(),
        avatar: None,
        workspace_path: None,
        model_profile_id: None,
        fallback_profile_ids: None,
        created_at: "2024-01-01 00:00:00".to_string(),
        updated_at: "2024-01-01 00:00:00".to_string(),
    }
}

/// 构造合法 NewAgent 基线（各用例只改关心的字段）
fn new_agent(provider: &str, api_key: &str) -> NewAgent {
    NewAgent {
        id: "test-agent".into(),
        name: "测试".into(),
        provider: provider.into(),
        model: "m".into(),
        system_prompt: String::new(),
        api_key: api_key.into(),
        base_url: None,
        temperature: 0.7,
        max_tokens: 16384,
        extra_params: None,
        sort_order: 0,
        cache_prompt: true,
        max_history_messages: None,
        context_window: None,
        enabled_tools: None,
        supports_vision: false,
        workspace_path: None,
        avatar: None,
        model_profile_id: None,
        fallback_profile_ids: None,
    }
}

#[test]
fn switch_base_url_follows_provider() {
    // 未换厂商：不注入默认地址（保持「不改」语义）
    assert_eq!(default_url_on_provider_switch(false, Some("glm")), None);
    // 换厂商：注册表默认地址（端点跟随厂商，防 DB 残留旧厂商 URL）
    assert_eq!(
        default_url_on_provider_switch(true, Some("glm")).as_deref(),
        Some("https://open.bigmodel.cn/api/paas/v4")
    );
    assert_eq!(
        default_url_on_provider_switch(true, Some("deepseek")).as_deref(),
        Some("https://api.deepseek.com")
    );
    // custom / 未知 provider 无默认（空串）→ None（保持不改）
    assert_eq!(default_url_on_provider_switch(true, Some("custom")), None);
    assert_eq!(default_url_on_provider_switch(true, Some("nope")), None);
    assert_eq!(default_url_on_provider_switch(true, None), None);
}

#[test]
fn base_url_absent_keeps_endpoint() {
    // absent + switch_url=None（未换厂商，或换入 custom 无默认）→ None（保持
    // 不改；回归：曾一律映射成 Some(None) 静默清空端点）
    assert_eq!(resolve_base_url_arg(None, None), None);
    // absent + 换厂商有默认 → Some(Some(默认))
    assert_eq!(
        resolve_base_url_arg(None, Some("https://api.deepseek.com")),
        Some(Some("https://api.deepseek.com"))
    );
    // 显式设值 → 照设
    assert_eq!(
        resolve_base_url_arg(Some(Some("https://x.example")), None),
        Some(Some("https://x.example"))
    );
    // 显式清空 → 照清（合法显式意图）
    assert_eq!(resolve_base_url_arg(Some(None), None), Some(None));
}

#[test]
fn agent_update_base_url_double_option_serde() {
    // 双层 Option 的 JSON 三形态（2026-09-07 补齐：base_url 曾缺
    // deserialize_double_option，null 与缺席同为「不改」，显式清空不可达——
    // resolve_base_url_arg 的 Some(None)=清空语义靠它才可达）：
    // 字段缺席 → None（不改）
    let absent: AgentUpdate = serde_json::from_str(r#"{"id":"a1"}"#).unwrap();
    assert_eq!(absent.base_url, None);
    // JSON null → Some(None)（清空）
    let nulled: AgentUpdate = serde_json::from_str(r#"{"id":"a1","base_url":null}"#).unwrap();
    assert_eq!(nulled.base_url, Some(None));
    // 值 → Some(Some(v))（设定）
    let valued: AgentUpdate =
        serde_json::from_str(r#"{"id":"a1","base_url":"https://api.deepseek.com"}"#).unwrap();
    assert_eq!(
        valued.base_url,
        Some(Some("https://api.deepseek.com".to_string()))
    );
}

#[test]
fn validate_allows_empty_key_for_ollama_and_custom() {
    // 免鉴权 provider：空 key 合法（空串仍会存 Stronghold 占位记录）
    assert!(validate_new_agent(&new_agent("ollama", "")).is_ok());
    assert!(validate_new_agent(&new_agent("custom", "")).is_ok());
}

#[test]
fn validate_rejects_empty_key_for_keyed_providers() {
    for p in [
        "openai",
        "glm",
        "glm-coding",
        "deepseek",
        "anthropic",
        "minimax",
        "minimax-cn",
    ] {
        let err = validate_new_agent(&new_agent(p, "  ")).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{p} 空 key 应被拒");
    }
    // 未知 provider 保守按需要 key 处理
    assert!(validate_new_agent(&new_agent("totally-unknown", "")).is_err());
}

#[test]
fn validate_accepts_keyed_provider_with_key() {
    assert!(validate_new_agent(&new_agent("anthropic", "sk-xxx")).is_ok());
}

#[test]
fn validate_rejects_missing_id_and_name() {
    let mut a = new_agent("ollama", "");
    a.id = "  ".into();
    assert!(matches!(
        validate_new_agent(&a),
        Err(AppError::Validation(_))
    ));
    let mut b = new_agent("ollama", "");
    b.name = String::new();
    assert!(matches!(
        validate_new_agent(&b),
        Err(AppError::Validation(_))
    ));
}

#[test]
fn validate_rejects_empty_provider_and_model() {
    let mut a = new_agent("ollama", "");
    a.provider = " ".into();
    assert!(matches!(
        validate_new_agent(&a),
        Err(AppError::Validation(_))
    ));
    let mut b = new_agent("ollama", "");
    b.model = "".into();
    assert!(matches!(
        validate_new_agent(&b),
        Err(AppError::Validation(_))
    ));
}

/// 物化闸：UI 完整输入可物化；旁路形态（需 Key 无 Key / custom 缺端点 /
/// 字段不齐）保持 legacy 不硬造 keyless 实体
#[test]
fn manual_materializable_gate() {
    // UI 路径：完整手动字段（keyed 厂商带 Key）
    assert!(manual_materializable(&new_agent("glm", "sk-1")));
    // 需 Key 厂商无 Key（提案旁路）→ 不物化
    assert!(!manual_materializable(&new_agent("glm", "  ")));
    // custom 缺端点 → 不物化；补端点 → 可
    let mut c = new_agent("custom", "");
    c.base_url = None;
    assert!(!manual_materializable(&c));
    c.base_url = Some("http://localhost:11434/v1".into());
    assert!(manual_materializable(&c));
    // ollama 免 Key 且有注册表默认端点 → 可
    assert!(manual_materializable(&new_agent("ollama", "")));
    // 字段不齐 → 不物化
    let mut p = new_agent("glm", "sk-1");
    p.provider = " ".into();
    assert!(!manual_materializable(&p));
    let mut m = new_agent("glm", "sk-1");
    m.model = "".into();
    assert!(!manual_materializable(&m));
}

#[tokio::test]
async fn mock_list_returns_seeded_agents() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);
    mock.seed(
        sample_agent_row("a2", "Agent 2"),
        "k2".into(),
        Some("https://api.example.com".into()),
    );

    let list = mock.list().await.unwrap();
    assert_eq!(list.len(), 2);
    let names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"Agent 1"));
    assert!(names.contains(&"Agent 2"));
}

#[tokio::test]
async fn mock_get_returns_correct_agent() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

    let a = mock.get("a1").await.unwrap();
    assert_eq!(a.id, "a1");
    assert_eq!(a.name, "Agent 1");

    // 不存在 → NotFound
    let err = mock.get("nonexistent").await.unwrap_err();
    match err {
        AppError::NotFound { resource, id } => {
            assert_eq!(resource, "agent");
            assert_eq!(id, "nonexistent");
        }
        e => panic!("expected NotFound, got {e:?}"),
    }
}

#[tokio::test]
async fn mock_get_with_credentials_returns_api_key_and_base_url() {
    let mock = MockAgentCmd::new();
    mock.seed(
        sample_agent_row("a1", "Agent 1"),
        "secret-key".into(),
        Some("https://api.example.com".into()),
    );

    let result = mock.get_with_credentials("a1").await.unwrap();
    assert_eq!(result.agent.id, "a1");
    assert_eq!(result.api_key, "secret-key");
    assert_eq!(result.base_url, Some("https://api.example.com".into()));
}

#[tokio::test]
async fn mock_create_adds_agent() {
    let mock = MockAgentCmd::new();
    let new = NewAgent {
        id: "test-agent".into(),
        name: "Test Agent".into(),
        provider: "anthropic".into(),
        model: "claude-3-5-sonnet".into(),
        system_prompt: "you are a helpful assistant".into(),
        api_key: "sk-test".into(),
        base_url: None,
        temperature: 0.7,
        max_tokens: 1024,
        extra_params: None,
        sort_order: 0,
        cache_prompt: true,
        max_history_messages: None,
        context_window: None,
        enabled_tools: None,
        supports_vision: false,
        workspace_path: None,
        avatar: None,
        model_profile_id: None,
        fallback_profile_ids: None,
    };

    let a = mock.create(new).await.unwrap();
    assert_eq!(a.name, "Test Agent");
    assert_eq!(mock.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn mock_update_modifies_name() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Old Name"), "k1".into(), None);

    let input = AgentUpdate {
        id: "a1".into(),
        name: Some("New Name".into()),
        provider: None,
        model: None,
        system_prompt: None,
        base_url: None,
        temperature: None,
        max_tokens: None,
        extra_params: None,
        sort_order: None,
        cache_prompt: None,
        max_history_messages: None,
        context_window: None,
        enabled_tools: None,
        supports_vision: None,
        workspace_path: None,
        avatar: None,
        model_profile_id: None,
        fallback_profile_ids: None,
    };

    let a = mock.update(input).await.unwrap();
    assert_eq!(a.name, "New Name");
}

#[tokio::test]
async fn mock_rotate_key_replaces_credentials() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Agent 1"), "old-key".into(), None);

    let input = RotateAgentKey {
        agent_id: "a1".into(),
        api_key: "new-key".into(),
        base_url: Some("https://new.example.com".into()),
    };
    mock.rotate_key(input).await.unwrap();

    let result = mock.get_with_credentials("a1").await.unwrap();
    assert_eq!(result.api_key, "new-key");
    assert_eq!(result.base_url, Some("https://new.example.com".into()));
}

#[tokio::test]
async fn mock_delete_removes_agent() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

    mock.delete("a1").await.unwrap();
    assert!(mock.get("a1").await.is_err());
    assert_eq!(mock.list().await.unwrap().len(), 0);
}

#[tokio::test]
async fn mock_call_log_records_operations() {
    let mock = MockAgentCmd::new();
    mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

    let _ = mock.list().await;
    let _ = mock.get("a1").await;
    let _ = mock.get_with_credentials("a1").await;
    let _ = mock.delete("a1").await;

    let log = mock.call_log();
    assert_eq!(log.len(), 4);
    assert_eq!(log[0], "list");
    assert_eq!(log[1], "get(a1)");
    assert_eq!(log[2], "get_with_credentials(a1)");
    assert_eq!(log[3], "delete(a1)");
}

/// 验证：SqlAgentCmd 与 MockAgentCmd 都实现了 AgentCmd，
/// 可被同一个函数以 trait object 形式接收（编译期检查）。
#[tokio::test]
async fn trait_object_works_for_both_impls() {
    async fn exercise(cmd: Arc<dyn AgentCmd>) -> AppResult<usize> {
        let list = cmd.list().await?;
        Ok(list.len())
    }

    let mock: Arc<dyn AgentCmd> = Arc::new(MockAgentCmd::new());
    let n = exercise(mock).await.unwrap();
    assert_eq!(n, 0);
}

/// 模板去雷回归：默认 agent.yaml 不得含活跃的 tool_max_rounds /
/// max_total_tokens 行——显式值是 B1 硬上限语义（触顶即停、不自动续期），
/// 写进模板会让所有新 agent 默认失去自动续期额度。
#[test]
fn default_yaml_template_comments_out_hard_caps() {
    let content = build_default_agent_yaml_content(
        "测试",
        "glm",
        "glm-5.2",
        None,
        0.7,
        4096,
        Some(&["read_file".to_string()]),
        None,
    );
    assert!(
        content.contains("# tool_max_rounds: 50"),
        "tool_max_rounds 应为注释行: {content}"
    );
    assert!(
        content.contains("# max_total_tokens: 3000000"),
        "max_total_tokens 应为注释行: {content}"
    );
    // 不得存在行首活跃（未注释）的两行
    for line in content.lines() {
        let trimmed = line.trim_start();
        assert!(
            !(trimmed.starts_with("tool_max_rounds:")
                || trimmed.starts_with("max_total_tokens:")),
            "不得有活跃硬上限行: {line}"
        );
    }
    // 常规字段照常生成
    assert!(content.contains("provider: glm"));
    assert!(content.contains("model: glm-5.2"));
    assert!(content.contains("max_tokens: 4096"));
    assert!(content.contains("- read_file"));
}

/// ModelProfile Phase 2：AgentUpdate.model_profile_id / fallback_profile_ids
/// 双层 Option JSON 三形态（base_url 惯例）。
#[test]
fn agent_update_model_profile_serde_three_forms() {
    // 字段缺席 → None（不改）
    let absent: AgentUpdate = serde_json::from_str(r#"{"id":"a1"}"#).unwrap();
    assert_eq!(absent.model_profile_id, None);
    assert_eq!(absent.fallback_profile_ids, None);

    // JSON null → Some(None)（解除引用 / 清链）
    let nulled: AgentUpdate = serde_json::from_str(
        r#"{"id":"a1","model_profile_id":null,"fallback_profile_ids":null}"#,
    )
    .unwrap();
    assert_eq!(nulled.model_profile_id, Some(None));
    assert_eq!(nulled.fallback_profile_ids, Some(None));

    // 值 → Some(Some(v))（设引用 / 设链）
    let valued: AgentUpdate = serde_json::from_str(
        r#"{"id":"a1","model_profile_id":"mp-1","fallback_profile_ids":["mp-2"]}"#,
    )
    .unwrap();
    assert_eq!(valued.model_profile_id, Some(Some("mp-1".into())));
    assert_eq!(
        valued.fallback_profile_ids,
        Some(Some(vec!["mp-2".to_string()]))
    );
}

/// 设引用与快照列族手填同批拒；解除引用与手填同批合法；链依赖主档。
#[test]
fn update_conflicts_profile_and_manual_fields_rejected() {
    let base = || AgentUpdate {
        id: "a1".into(),
        name: None,
        provider: None,
        model: None,
        system_prompt: None,
        base_url: None,
        temperature: None,
        max_tokens: None,
        extra_params: None,
        sort_order: None,
        cache_prompt: None,
        max_history_messages: None,
        context_window: None,
        enabled_tools: None,
        supports_vision: None,
        workspace_path: None,
        avatar: None,
        model_profile_id: None,
        fallback_profile_ids: None,
    };

    // 设引用 + provider 同批 → 拒
    let mut u = base();
    u.model_profile_id = Some(Some("mp-1".into()));
    u.provider = Some("glm".into());
    assert!(validate_update_model_fields_conflict(&u, false).is_err());

    // 设引用 + base_url 同批 → 拒（快照列族完整性）
    let mut u = base();
    u.model_profile_id = Some(Some("mp-1".into()));
    u.base_url = Some(Some("https://x.example".into()));
    assert!(validate_update_model_fields_conflict(&u, false).is_err());

    // 解除引用 + provider 同批 → 过（切回手动模式一并完成）
    let mut u = base();
    u.model_profile_id = Some(None);
    u.provider = Some("glm".into());
    u.model = Some("glm-5.3".into());
    assert!(validate_update_model_fields_conflict(&u, true).is_ok());

    // 引用态行 + 引用字段缺席（不改）+ 手填 model → 拒（提案卡批准
    // update_agent 直传快照族的形状——下轮解析覆盖回实体值，改了白改）
    let mut u = base();
    u.model = Some("glm-5.3".into());
    assert!(validate_update_model_fields_conflict(&u, true).is_err());

    // 同上但显式解除引用（Some(None)）→ 过（解除并改模型一次完成）
    let mut u = base();
    u.model_profile_id = Some(None);
    u.model = Some("glm-5.3".into());
    assert!(validate_update_model_fields_conflict(&u, true).is_ok());

    // legacy 行（无主档）手填 → 过（现状不变，非引用态无覆盖问题）
    let mut u = base();
    u.model = Some("glm-5.3".into());
    assert!(validate_update_model_fields_conflict(&u, false).is_ok());

    // legacy 行（无主档）设非空链 → 拒
    let mut u = base();
    u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
    assert!(validate_update_model_fields_conflict(&u, false).is_err());

    // 同批设主档 + 链 → 过
    let mut u = base();
    u.model_profile_id = Some(Some("mp-1".into()));
    u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
    assert!(validate_update_model_fields_conflict(&u, false).is_ok());

    // 已有主档的行只设链 → 过（沿用旧行主档）
    let mut u = base();
    u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
    assert!(validate_update_model_fields_conflict(&u, true).is_ok());

    // 解除引用 + 留非空链 → 拒（目标态无主档）
    let mut u = base();
    u.model_profile_id = Some(None);
    u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
    assert!(validate_update_model_fields_conflict(&u, true).is_err());

    // 清链 + 解除引用 → 过
    let mut u = base();
    u.model_profile_id = Some(None);
    u.fallback_profile_ids = Some(None);
    assert!(validate_update_model_fields_conflict(&u, true).is_ok());
}

/// 引用模式新建：跳过 provider/model/api_key 必填；链依赖主档。
#[test]
fn validate_new_agent_profile_mode_skips_manual_required() {
    let mut a = new_agent("", "");
    a.provider = String::new();
    a.model = String::new();
    a.api_key = String::new();
    // legacy 路径：空 provider 应拒
    assert!(validate_new_agent(&a).is_err());

    // 引用模式：同一入参全空合法（模型身份来自 profile）
    a.model_profile_id = Some("mp-1".into());
    assert!(validate_new_agent(&a).is_ok());

    // 引用模式但链无主档 → 拒
    let mut b = new_agent("", "");
    b.model_profile_id = None;
    b.fallback_profile_ids = Some(vec!["mp-2".into()]);
    assert!(validate_new_agent(&b).is_err());
}

/// Mock 双层 Option 语义：create 落引用列 / update 设与清。
#[tokio::test]
async fn mock_create_update_model_profile_fields() {
    let mock = MockAgentCmd::new();
    let mut new = new_agent("anthropic", "sk-xxx");
    new.model_profile_id = Some("mp-1".into());
    new.fallback_profile_ids = Some(vec!["mp-2".to_string(), "mp-3".to_string()]);
    let a = mock.create(new).await.unwrap();
    assert_eq!(a.model_profile_id.as_deref(), Some("mp-1"));
    assert_eq!(
        a.fallback_profile_ids,
        Some(vec!["mp-2".to_string(), "mp-3".to_string()])
    );

    // update：解除引用 + 清链
    let u = AgentUpdate {
        id: "test-agent".into(),
        model_profile_id: Some(None),
        fallback_profile_ids: Some(None),
        ..serde_json::from_str::<AgentUpdate>(r#"{"id":"test-agent"}"#).unwrap()
    };
    let a = mock.update(u).await.unwrap();
    assert_eq!(a.model_profile_id, None);
    assert_eq!(a.fallback_profile_ids, None);
}
