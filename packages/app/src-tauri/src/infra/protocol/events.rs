//! 事件 Payload — Tauri emit 给前端的 chat:* 事件负载
//!
//! 三簇：流式聊天事件（start/chunk/done/error/round-state/budget/retrying/
//! tool-call/thinking/summary）、工具授权（request/response）、配置提案。

use super::llm::TokenUsage;
use serde::{Deserialize, Serialize};

// =========================================================================
// 事件 Payload 结构
// =========================================================================

/// `chat:start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatStartPayload {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    /// 后端 materialize 后的用户消息 content_blocks（含附件提取出的 Text 块）。
    /// 仅当本次发送含 office/pdf 附件时为 Some——前端乐观用户消息只放了 Attachment
    /// 占位卡片、拿不到提取正文，据此就地 patch，让附件详情弹窗能展示提取原文。
    /// None（纯文本/图片消息）时前端不动用户消息。
    pub user_content_blocks: Option<String>,
}

/// `chat:assistant-start` 事件 payload
///
/// 多轮工具调用场景：每轮工具执行完毕、创建下一轮 assistant 占位消息时 emit。
/// 前端据此「冻结上一条 assistant」（把本轮 streaming 文本/思考/工具调用写入其
/// content_blocks，仅含 tool_use 不含 result）+「按 tool_use_id 组装 user(tool_result)
/// 插入」+「重置 streaming 状态」+「push 新 assistant 占位」。
///
/// 与 `chat:start` 区别：`chat:start` 在整次发送开始时由 chat_cmd 发一次（首条
/// assistant）；`chat:assistant-start` 在每轮工具后发（第 2 条及之后的 assistant）。
#[derive(Clone, Serialize)]
pub struct ChatAssistantStartPayload {
    pub conversation_id: String,
    pub message_id: String,
}

/// `chat:delegation-started` 事件 payload（MA-1 UX：运行中即可达）
///
/// 委派子会话**创建成功即发**（run_agent_turn spawn 前，inline）——此前
/// child_conversation_id 只在完成时的 tool_result 里回传，运行中的委派卡片/
/// 任务入口全都跳不进去。前端据此刷新会话列表（子会话行即刻可见，任务胶囊/
/// 运行中卡片即可跳转）。v1 串行执行保证同父同时至多一个运行中委派。
#[derive(Clone, Serialize)]
pub struct DelegationStartedPayload {
    /// 父会话 id（事件路由用）
    pub conversation_id: String,
    /// 新建的委派子会话 id
    pub child_conversation_id: String,
    /// 专家 agent 显示名
    pub agent_name: String,
    /// 子会话标题（task 截断文本，UX #4 已去「委派: 」前缀）
    pub title: String,
}

/// `chat:chunk` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatChunkPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub delta: String,
}

/// `chat:done` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatDonePayload {
    pub conversation_id: String,
    pub message_id: String,
    pub finish_reason: String,
    /// P2-3: Token 用量信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// `chat:error` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatErrorPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub kind: String,
    pub message: String,
}

/// `chat:round-state` 事件 payload — W2.4 可观测性
#[derive(Clone, Serialize, Debug)]
pub struct ChatRoundStatePayload {
    pub conversation_id: String,
    pub round: u32,
    pub elapsed_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_completion: u32,
    pub cached_tokens: u32,
    pub retry_count: u32,
}

/// `chat:budget` 事件 payload — 会话级 token 预算可观测（前端 HUD / 续期 toast）。
///
/// 与 round-state 语义不同：round-state 是单轮性能快照，本 payload 是跨轮
/// 累计状态 + 续期变更。发射点：每轮 usage 累计后（renewed=false）+ 触顶自动
/// 续期时（renewed=true）+ budget_exceeded 终止前（终态）。
/// 不入 session-event-log（瞬态 UI 事件；静态预算快照已由 turn_context 落库）。
#[derive(Clone, Serialize, Debug)]
pub struct ChatBudgetPayload {
    pub conversation_id: String,
    /// 本轮 usage 累计后的计费口径 Σ(未命中全价 + 命中 1/10 + 输出全价)。
    /// 缓存折扣前为毛成本 Σ(prompt_i + completion_i)——按毛成本计量会提前
    /// 熔断高命中长任务（预算诚实化，见 budget::billed_tokens）。
    pub cumulative_tokens: u64,
    /// 累计缓存命中 Σ cached_i（HUD「缓存命中 X%」分子；规范语义，见 TokenUsage）
    pub cumulative_cached_tokens: u64,
    /// 累计总输入 Σ prompt_i（HUD 命中率分母；含命中部分，规范语义）
    pub cumulative_prompt_tokens: u64,
    /// 当前生效上限（续期后已抬升；= initial × (renewal_index + 1)）
    pub effective_cap: u64,
    /// 初始上限（= turn_context.budget_max_tokens）
    pub initial_cap: u64,
    /// 已发生的自动续期次数（0 起）
    pub renewal_index: u32,
    /// 续期额度（0 = agent.yaml 显式硬上限，不续期）
    pub max_renewals: u32,
    /// 本次事件是否因触顶续期（前端 toast 触发器）
    pub renewed: bool,
    /// 当前工具轮数（0 起，与 round-state 对齐）
    pub round: u32,
}

/// `chat:retrying` 事件 payload — 通知前端正在重试
#[derive(Clone, Serialize)]
pub struct ChatRetryingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
    /// W2.6: 重试原因（如 "network_error" / "server_error_5xx"）
    pub reason: String,
}

/// `chat:processing` 事件 payload — send_message 重处理阶段心跳
///
/// **不变式（CLAUDE.md 同步）**：60s 静默超时计时器假定「后端必有活动事件回报」，
/// 但 OCR / Pipeline / 写库横跨 chat:start 之前的多个串行阶段，多图 OCR 易超 60s。
/// 后端在每个串行步骤「进入 / 完成」时 emit 一次，前端收到即重置 60s 滑动窗口，
/// 让计时器真正反映后端活动状态，避免误判「已死」造成 sending 提前变 false。
///
/// 与 `chat:start/chunk/done` 的区别：本事件**不入 session-event-log**——它不是
/// 业务事实、只是心跳；不入日志、不入轨迹。瞬态 UI 事件走 LoopEmitter 通道，
/// 事实走 `harness::event_log`（分工固定，不混用）。
///
/// `stage` 取稳定词表（前端 i18n 用）：
/// - `"pipeline"`    Pipeline 整体进入/出口（send_message 链路主节点）
/// - `"ocr"`         ModalCapabilityStage OCR 阶段（`progress` 字段填 `(done, total)`）
///
/// 注：心跳只是「快速路径」的计时器重置；60s 静默超时的最终裁决走后端真相
/// 确认（`is_conversation_streaming` 命令）——埋点枚举不全的静默窗口由它兜底。
#[derive(Clone, Serialize)]
pub struct ChatProcessingPayload {
    pub conversation_id: String,
    /// 阶段词表（见 struct 注释），前端按 i18n 表翻译展示。
    pub stage: &'static str,
    /// 人类可读的阶段描述（中文为主，前端可覆写为本地化文案）。
    pub message: String,
    /// 进度（仅 OCR 阶段填，格式 `(done, total)`，0 起计）。其他阶段为 None。
    /// 序列化跳过 None 字段，与现有 Payload 一致。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<(u32, u32)>,
}

// === P2-1 工具调用事件 payload ===

/// `chat:tool-call-start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallStartPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub name: String,
}

/// `chat:tool-call-delta` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallDeltaPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub delta: String,
}

/// `chat:tool-call-end` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallEndPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
}

/// `chat:tool-result` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolResultPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
    /// 工具执行耗时（毫秒），含授权等待
    pub duration_ms: u64,
}

/// `chat:thinking` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatThinkingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
}

/// `chat:summary-injected` 事件 payload（M1.5 A3-4 滚动摘要）
///
/// 当 MemoryStage 触发摘要压缩后，通过此事件通知前端。
#[derive(Clone, Serialize)]
pub struct ChatSummaryInjectedPayload {
    pub conversation_id: String,
    pub summary_tokens: u32,
    pub original_count: u32,
    pub kept_count: u32,
}

// === A2-3 工具授权事件 payload ===

/// `chat:tool-auth-request` 事件 payload (Rust → Frontend)
///
/// 当工具调用需要用户确认授权（例如路径不在白名单）时，Rust 侧 emit 此事件，
/// 携带工具名 / 待访问路径 / 参数 / 唯一 request_id，前端弹窗后用同一
/// `request_id` 响应。
///
/// - `request_id`     唯一标识，前后端匹配响应用
/// - `tool_use_id`    LLM 端的工具调用 ID（用于工具结果回填）
/// - `tool_name`      工具名
/// - `file_path`      待访问的路径（可能为空，例如 list_directory 也适用）
/// - `arguments`      工具调用参数 JSON 字符串（前端展示用）
/// - `conversation_id` / `message_id` 与其它 chat:* 事件保持一致，便于前端过滤
/// - `reason`         触发原因（前端展示文案）
#[derive(Clone, Serialize)]
pub struct ToolAuthRequestPayload {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub file_path: String,
    pub arguments: String,
    pub conversation_id: String,
    pub message_id: String,
    pub reason: String,
}

/// 授权范围（#11 分层授权记忆）：用户在审批卡上选择的「允许」生效档位。
/// 默认 `Once`（仅本次）；`ThisDir`/`ThisTool` 记入会话级授权记忆，
/// 本会话内同范围不再询问（流结束即清，不跨会话持久）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthScope {
    /// 仅本次（等价旧行为：精确路径入会话记忆）
    #[default]
    Once,
    /// 此目录（含子目录）会话内免问；无路径工具退化为工具档
    ThisDir,
    /// 此工具会话内免问（Confirm 级工具唯一可用的扩围档）
    ThisTool,
}

/// `chat:tool-auth-response` 事件 payload (Frontend → Rust)
///
/// 前端弹窗后通过此事件把用户选择告诉 Rust 侧。
/// Rust 侧在 `tool_executor` 里用 `request_id` 匹配 oneshot 通道，
/// 据此决定执行工具还是把工具结果写为「拒绝授权」。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolAuthResponse {
    pub request_id: String,
    pub allowed: bool,
    /// 允许的生效范围（拒绝时忽略）；`#[serde(default)]` 兼容旧前端
    #[serde(default)]
    pub scope: AuthScope,
}

// === 配置提案事件 ===

/// 敏感度分级（贯穿所有阶段的调节阀）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityTier {
    /// 🟢 非敏感：改 agent 名/温度/system_prompt、enable MCP、设 workspace、改时区
    #[serde(rename = "low")]
    Low,
    /// 🟡 敏感：API Key、新建带工具的 agent、创建 MCP server、改 embedding 配置
    #[serde(rename = "medium")]
    Medium,
    /// 🔴 红线：删除、跨 agent 改动、提权、读回密钥明文（提案路径根本不受理）
    #[serde(rename = "redline")]
    Redline,
}

/// 提案动作（Phase 1 仅 agent 域 create/update）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProposalAction {
    /// 创建 agent（🟢 无工具 / 🟡 带 enabled_tools）
    CreateAgent {
        id: String,
        name: String,
        provider: String,
        model: String,
        /// 🔴 绝对不能填真实 key，只能是 "__SLOT__" 占位
        api_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_tools: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
    },
    /// 更新 agent（只能更新当前 agent 自己）
    UpdateAgent {
        agent_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        temperature: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_tokens: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        enabled_tools: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
        /// Word 文档样式偏好（用户口头偏好的原文；`Some("")` = 摘除 yaml 块）。
        /// 落地走 `set_agent_word_profile`（agent.yaml 纯文件旁路，D12 双轨承载）
        #[serde(skip_serializing_if = "Option::is_none")]
        word_style_profile: Option<String>,
    },
}

/// `chat:config-proposal` 事件 payload（Rust → Frontend）
#[derive(Clone, Serialize)]
pub struct ConfigProposalPayload {
    pub request_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub sensitivity: SensitivityTier,
    pub action: ProposalAction,
    /// 人类可读的提案摘要（agent 生成，前端展示用）
    pub summary: String,
}

/// `chat:config-proposal-response` 事件 payload（Frontend → Rust）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigProposalResponse {
    pub request_id: String,
    #[serde(rename = "decision")]
    pub decision: ProposalDecision,
}

/// 用户对提案的决定
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    /// 用户批准，前端已通过现有可信命令执行
    Approved,
    /// 用户修改后批准
    Modified {
        /// 被修改的字段名 → 新值（JSON string）
        #[serde(default)]
        changes: std::collections::HashMap<String, String>,
    },
    /// 用户拒绝
    Rejected {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// 提案/授权请求失效（超时/通道关闭）时 Rust→Frontend 通知。
///
/// 前端按 `request_id` 清除对应的待处理卡片/弹窗，避免用户对已失效请求
/// 操作（例如对已超时的提案点「批准」会真的创建 agent，但对话流已当它
/// 取消 → 状态分裂、留下孤儿 agent）。
///
/// 用于 `chat:config-proposal-cancel` 与 `chat:tool-auth-request-cancel` 两个事件。
#[derive(Clone, Serialize)]
pub struct PendingRequestCancelPayload {
    pub request_id: String,
    pub conversation_id: String,
    /// "timeout" | "cancelled" | "abort"
    pub reason: String,
}

/// `chat:tool-auth-responded` 事件 payload（Rust → Frontend）
///
/// 工具授权请求已被应答（前端卡片或系统 toast 按钮任一路径）。前端按
/// `request_id` 清除 pendingAuthRequests 条目——toast 按钮路径前端无乐观删，
/// 无此事件则条目残留到 120s 超时（emit 单一入口 harness/approval_toast.rs）。
#[derive(Clone, Serialize)]
pub struct ToolAuthRespondedPayload {
    pub request_id: String,
    pub allowed: bool,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_auth_request_payload_serde() {
        let p = ToolAuthRequestPayload {
            request_id: "req-1".into(),
            tool_use_id: "tc-1".into(),
            tool_name: "read_file".into(),
            file_path: "/etc/passwd".into(),
            arguments: r#"{"path":"/etc/passwd"}"#.into(),
            conversation_id: "c-1".into(),
            message_id: "m-1".into(),
            reason: "路径 '/etc/passwd' 不在白名单中".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["request_id"], "req-1");
        assert_eq!(parsed["tool_use_id"], "tc-1");
        assert_eq!(parsed["tool_name"], "read_file");
        assert_eq!(parsed["file_path"], "/etc/passwd");
        assert_eq!(parsed["conversation_id"], "c-1");
        assert_eq!(parsed["message_id"], "m-1");
        // 8 个字段
        assert_eq!(parsed.as_object().unwrap().len(), 8);
    }

    #[test]
    fn tool_auth_response_serde_roundtrip() {
        let r = ToolAuthResponse {
            request_id: "req-2".into(),
            allowed: true,
            scope: AuthScope::ThisDir,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"request_id":"req-2","allowed":true,"scope":"this_dir"}"#
        );

        let back: ToolAuthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, "req-2");
        assert!(back.allowed);
        assert_eq!(back.scope, AuthScope::ThisDir);

        // 旧前端缺 scope 字段 → serde default = Once（向后兼容）
        let legacy: ToolAuthResponse =
            serde_json::from_str(r#"{"request_id":"req-x","allowed":true}"#).unwrap();
        assert_eq!(legacy.scope, AuthScope::Once);

        // false 路径
        let r2 = ToolAuthResponse {
            request_id: "req-3".into(),
            allowed: false,
            scope: AuthScope::Once,
        };
        let json2 = serde_json::to_string(&r2).unwrap();
        let back2: ToolAuthResponse = serde_json::from_str(&json2).unwrap();
        assert!(!back2.allowed);
    }

    #[test]
    fn sensitivity_tier_serde() {
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Low).unwrap(),
            r#""low""#
        );
        assert_eq!(
            serde_json::to_string(&SensitivityTier::Medium).unwrap(),
            r#""medium""#
        );
        let tier: SensitivityTier = serde_json::from_str(r#""medium""#).unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
    }

    #[test]
    fn proposal_action_create_agent_serde() {
        let action = ProposalAction::CreateAgent {
            id: "test-id".into(),
            name: "Test Agent".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            api_key: "__SLOT__".into(),
            base_url: None,
            system_prompt: Some("You are helpful.".into()),
            temperature: Some(0.7),
            max_tokens: None,
            enabled_tools: None,
            workspace_path: None,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"create_agent""#));
        assert!(json.contains(r#""api_key":"__SLOT__""#));
        // 反序列化
        let back: ProposalAction = serde_json::from_str(&json).unwrap();
        match back {
            ProposalAction::CreateAgent { id, name, .. } => {
                assert_eq!(id, "test-id");
                assert_eq!(name, "Test Agent");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn proposal_action_update_agent_serde() {
        let action = ProposalAction::UpdateAgent {
            agent_id: "a1".into(),
            name: Some("Renamed".into()),
            provider: None,
            model: None,
            system_prompt: None,
            base_url: None,
            temperature: Some(0.3),
            max_tokens: None,
            enabled_tools: None,
            workspace_path: None,
            word_style_profile: Some("正文宋体小四".into()),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains(r#""action":"update_agent""#));
        // D12：偏好字段随事件透传（None 时 skip 不占位）
        assert!(json.contains(r#""word_style_profile":"正文宋体小四""#));
        let back: ProposalAction = serde_json::from_str(&json).unwrap();
        match back {
            ProposalAction::UpdateAgent { agent_id, .. } => {
                assert_eq!(agent_id, "a1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn config_proposal_response_serde() {
        let r = ConfigProposalResponse {
            request_id: "req-1".into(),
            decision: ProposalDecision::Approved,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""decision":"approved""#));

        let r2 = ConfigProposalResponse {
            request_id: "req-2".into(),
            decision: ProposalDecision::Rejected {
                reason: Some("不需要".into()),
            },
        };
        let json2 = serde_json::to_string(&r2).unwrap();
        assert!(json2.contains(r#""reason":"不需要""#));
    }
}
