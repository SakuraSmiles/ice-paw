// IcePaw 前端业务类型定义 — 仅保留当前使用的类型

// ============================================================================
// Agent
// ============================================================================

export interface Agent {
  id: string;
  name: string;
  provider: string;
  model: string;
  system_prompt: string;
  base_url: string | null;
  temperature: number;
  max_tokens: number;
  extra_params: Record<string, unknown>;
  sort_order: number;
  cache_prompt: boolean;
  max_history_messages?: number | null;
  context_window?: number | null;
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  description?: string;
  avatar?: string | null;
  workspace_path?: string | null;
  config_from_file?: boolean;
  created_at: string;
  updated_at: string;
  has_api_key: boolean;
}

/** agent.yaml 字段快照（get_agent_yaml_fields 返回；null = 未设置/默认自适应） */
export interface AgentYamlFields {
  max_total_tokens: number | null;
  tool_max_rounds: number | null;
  system_prompt: string | null;
}

export interface NewAgent {
  id: string;
  name: string;
  provider: string;
  model: string;
  api_key: string;
  base_url?: string;
  system_prompt?: string;
  temperature?: number;
  max_tokens?: number;
  extra_params?: string;
  cache_prompt?: boolean;
  max_history_messages?: number | null;
  context_window?: number | null;
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  workspace_path?: string;
  avatar?: string;
}

export interface AgentUpdate {
  id: string;
  name?: string;
  provider?: string;
  model?: string;
  system_prompt?: string;
  base_url?: string;
  temperature?: number;
  max_tokens?: number;
  extra_params?: string;
  cache_prompt?: boolean;
  max_history_messages?: number | null;
  context_window?: number | null;
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  workspace_path?: string | null;
  /** 头像：undefined=不改 / null=清空 / string=设定（双层 Option 语义） */
  avatar?: string | null;
}

// ============================================================================
// Provider（模型提供方目录——后端 PROVIDERS 注册表经 list_providers 下发，
// 前端唯一数据源，勿在组件内再硬编码 provider 列表）
// ============================================================================

/** 协议族：决定鉴权 header 与端点拼接规则 */
export type ProviderProtocolKind = "openai" | "anthropic";

export interface ProviderInfo {
  /** 注册名（存库值，如 "glm-coding"） */
  name: string;
  protocol: ProviderProtocolKind;
  /** 默认地址；custom 为空串（必须显式填 base_url） */
  default_url: string;
  /** 备选探测端点 [标签, 地址]——未显式填地址时按 [默认, ...备选] 顺序回退探测 */
  alt_urls: [string, string][];
  /** 展示名（下拉主行） */
  label: string;
  /** 补充说明（下拉副行） */
  note: string | null;
  /** 是否必须配 API Key（ollama/custom 为 false） */
  requires_key: boolean;
  /** 是否必须显式填写 API URL（仅 custom） */
  requires_base_url: boolean;
  /** API Key 申请页地址（免 key 厂商为 null）——表单「去申请」直达 */
  key_url: string | null;
  /** 隐藏条目：不进前端下拉（旧入口/已下线），存量 agent 编辑仍可解析 */
  hidden: boolean;
  /** 静态模型目录（起点参考；「拉取」拿实时列表，手输永远保留） */
  models: string[];
}

/** test_provider_connection 结果：探测失败不是命令失败（ok=false + error 行内展示） */
export interface ProviderConnectionResult {
  ok: boolean;
  model_count: number;
  models: string[];
  error: string | null;
  /** 实际走通的端点地址（多端点回退时可能是备选端点）——回填 API URL 固化下来 */
  matched_url: string | null;
}

// ============================================================================
// Conversation
// ============================================================================

export interface Conversation {
  id: string;
  agent_id: string;
  title: string;
  pinned: boolean;
  created_at: string;
  updated_at: string;
  toolsOverride?: Record<string, boolean> | null;
  project_id?: string | null;
  /** 会话类型（MA-1，后端 migration 45；旧数据后端已兜底 'chat'）：
   *  'chat'=用户↔agent · 'delegation'=agent 委派子会话（侧栏隐藏，经委派卡片/项目任务列表进入） */
  kind?: string;
  /** 委派子会话的父会话 id（kind='delegation' 时必有；后端深度=1 护栏保证父为 chat 会话） */
  parent_conversation_id?: string | null;
}

export interface NewConversation {
  agent_id: string;
  title?: string;
  project_id?: string | null;
}

// ============================================================================
// Project（项目维度）
// ============================================================================

/** 项目成员（project_agents 关联表） */
export interface ProjectAgent {
  project_id: string;
  agent_id: string;
  role: string;
  joined_at: string;
}

/**
 * 项目。后端 `Project` 用 `#[serde(flatten)]` 把 ProjectRow 字段展开到顶层，
 * 再附带 `agents`，故此处字段全部平铺。
 */
export interface Project {
  id: string;
  name: string;
  description: string;
  icon: string;
  sort_order: number;
  workspace_path?: string | null;
  theme_color?: string | null;
  /** 项目头像图片（base64 dataURL；null 走名字渐变兜底） */
  avatar?: string | null;
  archived: boolean;
  created_at: string;
  updated_at: string;
  agents?: ProjectAgent[];
}

/** 创建项目入参（含初始成员 agent_ids，role 默认 member） */
export interface NewProject {
  name: string;
  description?: string;
  icon?: string;
  workspace_path?: string;
  theme_color?: string;
  avatar?: string;
  agent_ids?: string[];
}

/** 更新项目入参（partial update；undefined = 不改 / null = 清空） */
export interface UpdateProject {
  id: string;
  name?: string;
  description?: string;
  icon?: string;
  workspace_path?: string | null;
  theme_color?: string | null;
  avatar?: string | null;
}

/** 项目上下文读取结果（get_project_context）。
 *  available=false = 未解析到默认工作区（防御分支，正常启动不触发——
 *  preferences 会自动初始化默认工作区）；两文件内容读失败降级空串。 */
export interface ProjectContext {
  available: boolean;
  dir?: string | null;
  project_md: string;
  conventions_md: string;
}

// ============================================================================
// MA-2 项目台账 / 项目轨迹 / 概览（纯只读派生：任务 ≡ kind='delegation' 会话）
// ============================================================================

/** 任务台账行（list_project_tasks）。终态推导见 utils/taskStatus.ts：
 *  running 由前端流式 overlay（streamingConvIds），done/failed 由 termination 分桶。 */
export interface ProjectTask {
  conv_id: string;
  title: string;
  /** 执行者（被委派的专家 agent；名字 agent store 解析，无 FK 语义） */
  executor_agent_id: string;
  /** 发起者（null ≡ 用户发起） */
  initiator_agent_id: string | null;
  /** 委派图边——父会话（跳转回父会话用） */
  parent_conversation_id: string | null;
  started_at: string;
  updated_at: string;
  /** 最后一条 turn_ended 落库时间（无 = 进行中/中断） */
  ended_at: string | null;
  termination: string | null;
  rounds: number | null;
}

/** 项目事件流行（list_project_events）：`SessionEvent` 同构 + 会话标注列
 *  （后端 serde(flatten) 使 JSON 与单会话事件完全一致，只是多两列）。 */
export type ProjectEvent = SessionEvent & {
  session_title: string;
  session_kind: string; // chat | delegation
};

/** 项目概览统计（get_project_overview）。open（进行中+中断）不单列——
 *  前端由 tasks_total - 三桶推得。 */
export interface ProjectAgentShare {
  agent_id: string;
  messages: number;
  /** token 估算合计（messages.token_count SUM；展示标 ≈） */
  tokens: number;
}

export interface ProjectOverview {
  chat_conversations: number;
  delegation_conversations: number;
  messages: number;
  tasks_total: number;
  tasks_done: number;
  tasks_failed: number;
  tasks_ended_other: number;
  last_activity_at: string | null;
  /** 成员消息占比（「成员分布」横条排行；名字/模型前端 agent store 解析） */
  agent_shares: ProjectAgentShare[];
}

// ============================================================================
// Message
// ============================================================================

export type MessageRole = "user" | "assistant" | "system";

export interface Message {
  id: string;
  conversation_id: string;
  role: MessageRole;
  content: string;
  content_blocks: string;
  token_count: number | null;
  error: string | null;
  created_at: string;
  rowid: number;
  model: string | null;
}

// ============================================================================
// 流式聊天事件 payload
// ============================================================================

export interface ChatStartPayload {
  conversation_id: string;
  user_message_id: string;
  assistant_message_id: string;
  /** 含附件时：后端 materialize 后的 content_blocks（含提取正文），用于 patch 乐观用户消息 */
  user_content_blocks?: string | null;
}

/** 多轮工具调用中，每轮工具后创建下一轮 assistant 占位时 emit */
export interface ChatAssistantStartPayload {
  conversation_id: string;
  message_id: string;
}

/** 委派子会话创建成功即 emit（运行中卡片/任务胶囊可达；前端据此刷新会话列表） */
export interface DelegationStartedPayload {
  /** 父会话 id */
  conversation_id: string;
  child_conversation_id: string;
  agent_name: string;
  title: string;
}

export interface ChatChunkPayload {
  conversation_id: string;
  message_id: string;
  delta: string;
}

export interface ChatDonePayload {
  conversation_id: string;
  message_id: string;
  finish_reason: string;
  usage?: { prompt_tokens: number; completion_tokens: number; cached_tokens: number };
}


/** send_message 重处理阶段心跳（后端 Pipeline / OCR / 写库期间 emit）。
 *  前端订阅 chat:processing 仅做 resetSendTimeout()——撑住 60s 静默
 *  超时窗口，避免多图串行 OCR 触发误判 sending=false。 */
export interface ChatProcessingPayload {
  conversation_id: string;
  /** 阶段词表：pipeline | ocr | db_write */
  stage: string;
  message: string;
  progress?: [number, number]; // [done, total]，仅 OCR 阶段填
}

export interface ChatErrorPayload {
  conversation_id: string;
  message_id: string;
  kind: string;
  message: string;
}

export interface ChatRetryingPayload {
  conversation_id: string;
  message_id: string;
  attempt: number;
  max_attempts: number;
  reason?: string;
}

export interface ChatRoundStatePayload {
  conversation_id: string;
  round: number;
  elapsed_ms: number;
  tokens_prompt: number;
  tokens_completion: number;
  cached_tokens: number;
  retry_count: number;
}

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; data: string; media_type: string }
  | { type: "tool_use"; id: string; name: string; input: string }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error?: boolean }
  | { type: "thinking"; thinking: string; signature?: string }
  | { type: "attachment"; name: string; kind: string; size: number }
  /** @ 引用卡（纯 UI；LLM 读后端 materialize 展开的 text 块） */
  | { type: "reference"; ref_kind: "conversation" | "agent" | "message"; target_id: string; display: string };

/**
 * 聊天文件附件（office/pdf）。后端在 send_message 入口 materialize：
 * 每个附件产出 `attachment`（UI 卡片元信息，不发给 LLM）+ `text`（提取正文，发 LLM）。
 * 与后端 `AttachedFile` 对齐。
 */
export interface AttachedFile {
  /** 文件名（含扩展名），决定后端解析格式 */
  name: string;
  /** base64 编码字节（不含 data URL 前缀） */
  data: string;
}

// ============================================================================
// session-events / 会话轨迹回放（与后端 harness::event_log 对齐）
// list_session_events 命令返回；event 按 kind 判别，payload 类型随之窄化。
// 详见 docs/backend-api-reference.md 与 harness/event_log.rs
// ============================================================================

/** 单条会话事件（list_session_events 返回元素；payload 已在服务端 parse） */
export interface SessionEventBase {
  id: number;
  session_id: string;
  seq: number; // seq 正序 = 权威回放序
  actor: string; // "user" 或 "agent:<agent_id>"
  turn_id: string | null;
  message_id: string | null;
  created_at: string;
}
export type SessionEvent =
  | (SessionEventBase & { kind: "turn_context"; payload: TurnContextPayload })
  | (SessionEventBase & { kind: "user_message"; payload: UserMessagePayload })
  | (SessionEventBase & { kind: "assistant_message"; payload: AssistantMessagePayload })
  | (SessionEventBase & { kind: "tool_execution"; payload: ToolExecutionPayload })
  | (SessionEventBase & { kind: "tool_result_message"; payload: ToolResultMessagePayload })
  | (SessionEventBase & { kind: "attachment_stored"; payload: AttachmentStoredPayload })
  | (SessionEventBase & { kind: "summary_created"; payload: SummaryPayload })
  | (SessionEventBase & { kind: "summary_updated"; payload: SummaryPayload })
  | (SessionEventBase & { kind: "message_error"; payload: MessageErrorPayload })
  | (SessionEventBase & { kind: "message_discarded"; payload: MessageDiscardedPayload })
  | (SessionEventBase & { kind: "turn_ended"; payload: TurnEndedPayload })
  | (SessionEventBase & { kind: "modal_adapted"; payload: ModalAdaptedPayload })
  | (SessionEventBase & { kind: "hook_injected"; payload: HookInjectedPayload })
  | (SessionEventBase & { kind: "plan_updated"; payload: PlanUpdatedPayload });

export interface TurnContextPayload {
  v?: number;
  provider: string;
  effective_model: string;
  model_override?: string | null;
  tools_enabled: boolean;
  tool_names: string[];
  temperature?: number | null;
  max_tokens?: number | null;
  tool_max_rounds?: number | null;
  budget_max_tokens?: number | null;
  context_window?: number | null;
}
/** `chat:budget` 事件 payload — 会话级 token 预算状态（HUD / 续期 toast） */
export interface ChatBudgetPayload {
  conversation_id: string;
  /** 本轮 usage 累计后的计费口径 Σ(未命中全价 + 命中 1/10 + 输出全价) */
  cumulative_tokens: number;
  /** 累计缓存命中 Σ cached_i（「缓存命中 X%」分子；规范语义含命中部分） */
  cumulative_cached_tokens: number;
  /** 累计总输入 Σ prompt_i（命中率分母；含命中部分，规范语义） */
  cumulative_prompt_tokens: number;
  /** 当前生效上限（续期后已抬升） */
  effective_cap: number;
  /** 初始上限（= turn_context.budget_max_tokens） */
  initial_cap: number;
  /** 已发生的自动续期次数（0 起） */
  renewal_index: number;
  /** 续期额度（0 = 显式硬上限不续期） */
  max_renewals: number;
  /** 本次事件是否因触顶续期（toast 触发器） */
  renewed: boolean;
  /** 当前工具轮数（0 起） */
  round: number;
}
export interface UserMessagePayload { v?: number; content: string; blocks: ContentBlock[]; }
export interface AssistantMessagePayload {
  v?: number;
  model?: string | null;
  content: string;
  blocks: ContentBlock[];
  token_count?: number | null;
  duration_ms?: number | null; // 本轮生成耗时（毫秒；事件纪元早期无此字段）
  round: number; // 0 起
  continuation: boolean; // 自动续写
}
export interface ToolExecutionPayload {
  v?: number;
  tool_call_id: string;
  tool_use_id?: string | null;
  tool_name: string;
  arguments: string; // JSON 字符串
  result?: string | null;
  is_error: boolean;
  duration_ms: number;
}
export interface ToolResultMessagePayload { v?: number; blocks: ContentBlock[]; }
export interface AttachmentPageItem { idx: number; name: string; kind: string; label: string; token_est: number; }
export interface AttachmentBytesItem { idx: number; name: string; ext: string; bytes_len: number; }
/** 后端 tag="kind" 判别枚举（rename snake_case） */
export type AttachmentStoredPayload =
  | { kind: "pages"; v?: number; items: AttachmentPageItem[] }
  | { kind: "bytes"; v?: number; items: AttachmentBytesItem[] };
export interface SummaryPayload {
  v?: number;
  summary_message_id: string;
  content: string;
  covered_until_rowid: number;
  /** 覆盖终点消息的首现事件 seq（Phase 2B 阶段 2 起主锚；旧事件无此字段 → undefined） */
  covered_until_seq?: number;
}
export interface MessageErrorPayload { v?: number; kind: string; error: string; }
export interface MessageDiscardedPayload { v?: number; reason: string; }
export interface TokenUsage { prompt_tokens: number; completion_tokens: number; cached_tokens: number; }
export interface TurnEndedPayload {
  v?: number;
  termination: string; // stop|length|max_tokens|tool_use|budget_exceeded|stuck|abort|error|interrupted(boot 自愈补记：进程死亡时 turn 中断)
  rounds: number;
  usage?: TokenUsage | null;
  user_token_count?: number | null;
}
export interface ModalAdaptedItem { index: number; outcome: string; ocr_text?: string | null; }
export interface ModalAdaptedPayload {
  v?: number;
  stage: string; // user_image|tool_image|history
  mode: string; // vision_passthrough|ocr_substitute|strip_to_marker
  items: ModalAdaptedItem[];
}
export interface HookInjectedPayload { v?: number; point: string; prompt: string; } // point: conversation_start|before_llm

/** 计划条目（plan_updated 事件与 get_session_plan 快照共用形状）。
 *  计划=意图文档（会话内容），与任务（委派会话，执行单元）正交——
 *  task_conversation_id 是「声明→执行」的引用边，勾选恒为 agent 判断。 */
export interface PlanItem {
  text: string;
  status: string; // pending | in_progress | done
  task_conversation_id?: string | null;
}

/** plan_updated 事件 payload（全量快照，回放 last-wins 取最后一条 = 当前计划） */
export interface PlanUpdatedPayload { v?: number; items: PlanItem[]; }

/** get_session_plan 命令返回（当前计划快照 + 落库时间） */
export interface PlanSnapshot { items: PlanItem[]; updated_at: string; }

/** list_turn_anchors 命令返回（UX #5 轮次导航条）：一轮 = 一条用户消息，
 *  轮号 = 下标 +1（与轨迹页「第 N 轮」同基准） */
export interface TurnAnchor { message_id: string; preview: string; created_at: string; }

export interface ChatToolCallStartPayload {
  conversation_id: string; message_id: string; id: string; name: string;
}
export interface ChatToolCallDeltaPayload {
  conversation_id: string; message_id: string; id: string; delta: string;
}
export interface ChatToolCallEndPayload {
  conversation_id: string; message_id: string; id: string;
}
export interface ChatToolResultPayload {
  conversation_id: string; message_id: string; tool_use_id: string; content: string; is_error: boolean;
  duration_ms: number;
}
export interface ChatThinkingPayload {
  conversation_id: string; message_id: string; content: string;
}
export interface ChatSummaryInjectedPayload {
  conversation_id: string; summary_tokens: number; original_count: number; kept_count: number;
}

// ============================================================================
// 工具授权事件
// ============================================================================

export interface ToolAuthRequestPayload {
  request_id: string; tool_use_id: string; tool_name: string; file_path: string;
  arguments: string; conversation_id: string; message_id: string; reason: string;
}
/** 委派授权请求（chat:delegation-auth-request）：delegate_to_agent 建子会话前
 *  弹卡——目标 agent + 任务全文 + 预授权档。与工具授权共用 oneshot 应答通道
 *  （respond_tool_auth / ToolAuthResponse）。判别：payload 有 agent_name 字段。*/
export interface DelegationAuthRequestPayload {
  request_id: string; conversation_id: string; message_id: string;
  agent_name: string; agent_id: string; task: string;
}
/** #11 分层授权范围：once=仅本次（默认）/ this_dir=此目录含子目录（会话内）/
 *  this_tool=此工具（会话内，Confirm 级工具唯一扩围档）。与后端 AuthScope 对齐。*/
export type AuthScope = "once" | "this_dir" | "this_tool";
/** 委派预授权档（两档拍板 2026-09-03）：commands=命令免问（预授子会话
 *  run_command 工具档）；缺省/undefined = 逐次审批。与后端 DelegationGrant 对齐。*/
export type DelegationGrant = "commands";
export interface ToolAuthResponse {
  request_id: string; allowed: boolean; scope?: AuthScope;
  delegation_grant?: DelegationGrant;
}
/** store 侧待处理授权条目：payload + 前端收到时刻（120s 倒计时显示用）。
 *  payload 联合：工具授权 | 委派授权（判别用 "agent_name" in payload）。*/
export interface PendingAuthEntry {
  payload: ToolAuthRequestPayload | DelegationAuthRequestPayload;
  receivedAt: number;
}

// ============================================================================
// 屏幕共享通道（computer-use 批次④：授权与可见性的单位）
// ============================================================================

/** 附着会话（screen:channel-state 负载形态，键值拍平；与后端 AttachedConv 对齐） */
export interface ScreenChannelAttached {
  conv_id: string;
  agent_name: string;
  conv_title: string;
  purpose: string;
}

/** 通道单一全量事件负载（§4.9：形状跨步骤稳定）。步骤 1 只有 status/opened_at/
 *  attached 真生效；paused/hud_monitor/holder/queue/human_active/writing 随
 *  步骤 2-4 逐步点亮（协议从第一天定型，前端不随步骤演进改形状）。 */
export interface ScreenChannelState {
  status: "off" | "active";
  paused: boolean;
  opened_at: number | null;
  hud_monitor: number;
  attached: ScreenChannelAttached[];
  holder: string | null;
  queue: string[];
  human_active: boolean;
  /** 写件执行中（B7 写操作避让：HUD 自动收缩 + 点击穿透） */
  writing: boolean;
  screenshot_count: number;
}

// ============================================================================
// 配置提案事件
// ============================================================================

export type SensitivityTier = "low" | "medium" | "redline";

export interface ProposalActionCreateAgent {
  action: "create_agent";
  id: string;
  name: string;
  provider: string;
  model: string;
  api_key: string;  // 总是 "__SLOT__"
  base_url?: string | null;
  system_prompt?: string | null;
  temperature?: number | null;
  max_tokens?: number | null;
  enabled_tools?: string[] | null;
  workspace_path?: string | null;
}

export interface ProposalActionUpdateAgent {
  action: "update_agent";
  agent_id: string;
  name?: string | null;
  provider?: string | null;
  model?: string | null;
  system_prompt?: string | null;
  base_url?: string | null;
  temperature?: number | null;
  max_tokens?: number | null;
  enabled_tools?: string[] | null;
  workspace_path?: string | null;
  /** Word 文档样式偏好（自由文字块；空串 = 摘除 yaml 块） */
  word_style_profile?: string | null;
}

export type ProposalAction = ProposalActionCreateAgent | ProposalActionUpdateAgent;

export interface ConfigProposalPayload {
  request_id: string;
  conversation_id: string;
  message_id: string;
  tool_use_id: string;
  sensitivity: SensitivityTier;
  action: ProposalAction;
  summary: string;
}

export interface ConfigProposalResponse {
  request_id: string;
  decision: "approved" | "modified" | "rejected";
  changes?: Record<string, string>;
  reason?: string;
}

// ============================================================================
// UserPreferences
// ============================================================================

export interface UserPreferences {
  default_agent_id?: string | null;
  default_template_id?: string | null;
  on_startup?: string;
  language?: string;
  theme?: string;
  code_theme?: string;
  font_size?: number;
  default_provider?: string | null;
  send_shortcut?: string | null;
  auto_scroll?: boolean | null;
  auto_render?: boolean | null;
  auto_timestamp?: boolean | null;
  keyboard_shortcuts?: Record<string, string> | null;
  timezone?: string;
  default_workspace_path?: string | null;
  embedding_provider?: string;
  embedding_model?: string;
  embedding_api_key?: string;
  embedding_base_url?: string;
  /** Phase B 视觉读取（旧四键，仅读侧兼容回落——vision_config 缺失时拼成单条目） */
  vision_provider?: string;
  vision_model?: string;
  vision_api_key?: string;
  vision_base_url?: string;
  /** 视觉读取条目链（两档制）：Some=新格式权威（[] = 显式清空）；null/undefined=回落旧四键 */
  vision_config?: VisionConfigEntry[] | null;
}

/** 视觉读取配置的一个条目（主模型或降级模型，按序尝试、首个成功即用） */
export interface VisionConfigEntry {
  provider: string;
  model: string;
  api_key: string;
  /** 自定义端点（可选；空=按 provider 推导官方 OpenAI 兼容端点） */
  base_url?: string | null;
}

// ============================================================================
// MCP Server
// ============================================================================

export type McpTrustLevel = "trusted" | "untrusted";

/** MCP 传输类型：stdio（本地子进程）/ http（streamable HTTP）/ sse（Server-Sent Events） */
export type McpTransport = "stdio" | "http" | "sse";

/** MCP Server 配置（与后端 McpServerConfig 对齐） */
export interface McpServer {
  id: string;
  name: string;
  description: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
  trust_level: McpTrustLevel;
  scope: string;
  /** 运行时类型：system 走系统 PATH（npx），bundled 用内置 node + 预打包包 */
  runtime_kind: "system" | "bundled";
  /** 传输类型：stdio / http / sse（默认 stdio）；顶层路由，runtime_kind 仅 stdio 时生效 */
  transport: McpTransport;
  /** http/sse 远程端点 URL（stdio 时为 null） */
  url: string | null;
  /** http/sse 自定义请求头（如 Authorization），stdio 时为空对象 */
  headers: Record<string, string>;
  /** OpenAI 合规命名空间索引：工具名 = `t{tool_index}_{tool}`；后端自动分配、不可变 */
  tool_index?: number;
  created_at: string;
  updated_at: string;
}

/** MCP Server 运行时状态快照（后端 ServerSnapshot 对齐） */
export interface McpServerSnapshot {
  id: string;
  name: string;
  description: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  enabled: boolean;
  trust_level: McpTrustLevel;
  scope: string;
  /** 运行时类型：system 走系统 PATH（npx），bundled 用内置 node + 预打包包 */
  runtime_kind: "system" | "bundled";
  /** 传输类型：stdio / http / sse */
  transport: McpTransport;
  /** http/sse 远程端点 URL（stdio 时为 null） */
  url: string | null;
  /** http/sse 自定义请求头 */
  headers: Record<string, string>;
  /** OpenAI 合规命名空间索引（见 McpServer.tool_index） */
  tool_index?: number;
  /** 运行时状态 */
  status: "disabled" | "starting" | "running" | "failed";
  /** running 时的工具数 */
  tool_count: number | null;
  /** running 时的工具列表 */
  tools: McpToolDef[] | null;
  /** failed 时的错误信息 */
  error: string | null;
  created_at: string;
  updated_at: string;
}

/** 创建 MCP Server 入参 */
export interface NewMcpServer {
  id: string;
  name: string;
  description?: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  enabled?: boolean;
  trust_level?: McpTrustLevel;
  scope?: string;
  /** 传输类型，默认 stdio */
  transport?: McpTransport;
  /** http/sse 远程端点 URL */
  url?: string | null;
  /** http/sse 自定义请求头 */
  headers?: Record<string, string>;
}

/** 更新 MCP Server 入参 */
export interface McpServerUpdate {
  id: string;
  name?: string;
  description?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  enabled?: boolean;
  trust_level?: McpTrustLevel;
  scope?: string;
  transport?: McpTransport;
  url?: string | null;
  headers?: Record<string, string>;
}

/** MCP Server 提供的工具定义（来自 tools/list） */
export interface McpToolDef {
  name: string;
  description: string;
  input_schema: unknown;
}

// =========================================================================
// 知识库（RAG v1，约定单库模型：global / agent 级别各一个，directory 系统推导）
// =========================================================================

/** 知识库（按级别约定存在） */
export interface Kb {
  id: string;
  name: string;
  /** 'agent' | 'project' | 'global' */
  scope: string;
  /** agent_id / project_id；global 时为 null */
  owner_id: string | null;
  /** 监听的知识库目录绝对路径（系统按约定推导，不让用户填） */
  directory: string;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

/** 知识库文档（索引项） */
export interface KbDocument {
  id: string;
  kb_id: string;
  /** 相对 kb.directory 的路径 */
  file_path: string;
  title: string;
  summary: string;
  /** JSON 数组字符串，如 '["rust","tauri"]' */
  tags: string;
  content_hash: string | null;
  file_mtime: string | null;
  indexed_at: string;
}

/** 重建索引的统计 */
export interface IndexStats {
  indexed: number;
  skipped: number;
  deleted: number;
}

/** 某 KB 的统计（文档数 + chunk 向量进度） */
export interface KbStats {
  total_documents: number;
  total_chunks: number;
  embedded_chunks: number;
}

/** 全量重建 embedding 的统计 */
export interface RebuildStats {
  kbs: number;
  chunks: number;
}
