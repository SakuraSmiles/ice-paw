// IcePaw 前端业务类型定义
// 单一来源（Single Source of Truth）：所有 Tauri Commands 的入参与返回类型均在此声明。
// 业务组件、Pinia store 与 src/api/bridge.ts 共同消费本文件，保证 Rust 侧结构与 TS 侧一致。
//
// 注意：本文件不包含敏感字段（例如 api_key 明文）。Rust 侧 commands/*.rs 会将敏感字段
// 存储到 stronghold vault，前端业务层永不接触明文。

// ============================================================================
// Agent（前端视图，不含 api_key）
// ============================================================================

/**
 * Agent 实体：对应数据库 `agents` 表中一行记录。
 *
 * 字段说明：
 * - id            主键（UUID 字符串，由 Rust 侧生成）
 * - name          用户自定义名称（例如「论文润色」「代码评审」）
 * - provider      LLM 提供商标识（openai / anthropic / minimax / minimax-cn / deepseek / glm ...）
 * - model         具体模型名（例如 gpt-4o / claude-sonnet-4-20250514 / minimax-cn/M3 / deepseek-chat / glm-4.7）
 * - system_prompt 系统提示词（对话前固定注入的 prompt）
 * - base_url      自定义 API 地址，可为空（使用 provider 默认地址）
 * - temperature   采样温度（0-2 之间，浮点）
 * - max_tokens    单次回复最大 token 数（整数）
 * - extra_params  额外参数 JSON 字符串（透传给 provider 的额外字段）
 * - sort_order    列表排序权重（值小者靠前）
 * - created_at    创建时间（ISO 8601 字符串）
 * - updated_at    最近更新时间（ISO 8601 字符串）
 *
 * 注意：api_key 不在此结构中，前端通过 bridge.agents.rotateKey() 单独投递。
 */
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
  /** P2-3: 是否启用 prompt caching */
  cache_prompt: boolean;
  /**
   * A3-2: 历史消息窗口上限。
   * - `null` / `undefined` 表示使用系统默认（20 条）
   * - `N` (N > 0) 表示最多加载最近 N 条历史消息注入 LLM 上下文
   *
   * 不同 Agent 可拥有不同上下文长度（8K 上下文用 16 条，200K 上下文用 60 条）。
   */
  max_history_messages?: number | null;
  /**
   * Task 4: 工具白名单。
   * - `null` / `undefined` = 全部启用（向后兼容）
   * - `string[]` = 仅启用列出的工具
   * - `[]`（空数组）= 全部禁用
   */
  enabled_tools?: string[] | null;
  /** 是否支持图片输入 */
  supports_vision?: boolean;
  created_at: string;
  updated_at: string;
  hasApiKey: boolean;
}

/**
 * 创建 Agent 时的入参。
 *
 * 与 Agent 的差异：
 * - 不含 id（由 Rust 侧生成）
 * - 不含时间戳（由 Rust 侧填默认值）
 * - 不含 sort_order（由 Rust 侧默认 0，后续可由 update 接口调整）
 * - 必须显式传入 api_key（仅此入口接收明文，之后 Rust 侧加密入 vault）
 */
export interface NewAgent {
  name: string;
  provider: string;
  model: string;
  api_key: string;
  base_url?: string;
  system_prompt?: string;
  temperature?: number;
  max_tokens?: number;
  extra_params?: string;
  /** P2-3: 是否启用 prompt caching（默认 true） */
  cache_prompt?: boolean;
  /** A3-2: 历史消息窗口上限（null/undefined = 使用系统默认） */
  max_history_messages?: number | null;
  /** Task 4: 工具白名单（null/undefined = 全部启用） */
  enabled_tools?: string[] | null;
  /** 是否支持图片输入（默认 false） */
  supports_vision?: boolean;
}

/**
 * 更新 Agent 时的入参（partial update）。
 *
 * 字段语义：
 * - id        必传，用于定位记录
 * - 其余字段均可选；不传的字段在 Rust 侧保持原值
 *
 * 注意：api_key 不在此结构中，更换 key 请走 bridge.agents.rotateKey()。
 */
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
  /** P2-3: 是否启用 prompt caching */
  cache_prompt?: boolean;
  /**
   * A3-2: 历史消息窗口上限。
   * - 不传 → 不更新（保留原值）
   * - 传 null → 清空，恢复系统默认
   * - 传 N → 设成 N
   */
  max_history_messages?: number | null;
  /**
   * Task 4: 工具白名单。
   * - 不传 → 不更新（保留原值）
   * - 传 null → 清空，恢复为全部启用
   * - 传 string[] → 仅启用列出的工具
   */
  enabled_tools?: string[] | null;
  /** 是否支持图片输入 */
  supports_vision?: boolean;
}

// ============================================================================
// Conversation（会话）
// ============================================================================

/**
 * 会话实体：对应数据库 `conversations` 表中一行记录。
 *
 * 字段说明：
 * - id         主键
 * - agent_id   关联的 Agent 外键
 * - title      会话标题（首条消息前 50 字自动生成，或用户手动重命名）
 * - pinned     是否置顶（列表排序中 pinned DESC 优先）
 * - created_at 创建时间
 * - updated_at 最近活动时间（任何消息写入都会刷新该字段）
 */
export interface Conversation {
  id: string;
  agent_id: string;
  title: string;
  pinned: boolean;
  created_at: string;
  updated_at: string;
}

/**
 * 创建会话的入参。
 *
 * - agent_id 必传，关联到具体 Agent
 * - title    可选；不传则由 Rust 侧默认空串，待首条消息写入时再补
 */
export interface NewConversation {
  agent_id: string;
  title?: string;
}

// ============================================================================
// Message（消息）
// ============================================================================

/**
 * 消息角色枚举。
 *
 * - user       用户消息
 * - assistant 助手回复
 * - system    系统注入消息（一般由 Rust 侧补入 system prompt，前端不会主动发送）
 */
export type MessageRole = "user" | "assistant" | "system";

/**
 * 消息实体：对应数据库 `messages` 表中一行记录。
 *
 * 字段说明：
 * - id              主键
 * - conversation_id 所属会话外键
 * - role            角色（见 MessageRole）
 * - content         正文文本（兼容旧消息）
 * - content_blocks  P2-1: JSON 数组字符串（ContentBlock[]），空数组表示纯文本消息
 * - token_count     消耗 token 统计（可空）
 * - error           错误信息
 * - created_at      创建时间
 * - rowid           SQLite 物理行号，用于向上翻页的复合游标
 *                   `(created_at, rowid)`。前端不展示该字段。
 */
export interface Message {
  id: string;
  conversation_id: string;
  role: MessageRole;
  content: string;
  /** P2-1: JSON 数组字符串，解析后为 ContentBlock[] */
  content_blocks: string;
  token_count: number | null;
  error: string | null;
  created_at: string;
  /** 分页游标：与后端 MessageRow.rowid 对齐，前端不展示 */
  rowid: number;
}

/**
 * 创建消息的入参。
 *
 * - conversation_id 必传
 * - role           必传（字符串，便于前端扩展自定义角色，Rust 侧会做白名单校验）
 * - content        必传
 * - token_count    可选，一般由 Rust 侧在流式完成后回填，不传默认 null
 */
export interface NewMessage {
  conversation_id: string;
  role: string;
  content: string;
  token_count?: number;
}

// ============================================================================
// 流式聊天事件 payload（与 Rust ChatStartPayload / ChatChunkPayload 等对齐）
// ============================================================================

/**
 * `chat:start` 事件 payload
 *
 * Rust 侧在 `send_message` 命令接收到、写入用户与助手两条占位消息、注册
 * CancellationToken 之后立即 emit。`user_message_id` 与 `assistant_message_id`
 * 是 Rust 新生成的真实 ID，前端可以用它们校正本地乐观插入的临时消息。
 */
export interface ChatStartPayload {
  conversation_id: string;
  user_message_id: string;
  assistant_message_id: string;
}

/**
 * `chat:chunk` 事件 payload
 *
 * 每个 SSE 增量都会触发一次；`message_id` 即助手消息的 ID。
 */
export interface ChatChunkPayload {
  conversation_id: string;
  message_id: string;
  delta: string;
}

/**
 * `chat:done` 事件 payload
 *
 * 流正常结束时触发；`finish_reason` 由 provider 给出（常见值：
 *   - "stop"              正常结束
 *   - "length"            达到 max_tokens 上限
 *   - "abort"             取消（用户调用 stop_generation）
 *   - "budget_exceeded"    Token 预算超限（W4.2）
 *   - "tool_use"          工具轮数上限（MAX_TOOL_ROUNDS）
 *   - "stuck"             LLM 连续多轮无进展，自动终止（M2.1）
 * ）。
 */
export interface ChatDonePayload {
  conversation_id: string;
  message_id: string;
  finish_reason: string;
  /** P2-3: Token 用量 */
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    cached_tokens: number;
  };
}

/**
 * `chat:error` 事件 payload
 *
 * 任意阶段错误（HTTP 失败 / SSE 解析失败 / provider 异常 / 取消）。
 * `kind` 是错误大类（"llm" / "stream" / "cancelled" / "internal" 等），
 * `message` 是可读错误描述。
 */
export interface ChatErrorPayload {
  conversation_id: string;
  message_id: string;
  kind: string;
  message: string;
}

/**
 * `chat:retrying` 事件 payload
 *
 * LLM 流式中断后自动重试时触发。
 * `attempt` 是当前重试序号（从 1 开始），`max_attempts` 是最大重试次数。
 */
export interface ChatRetryingPayload {
  conversation_id: string;
  message_id: string;
  attempt: number;
  max_attempts: number;
  /** W2.6: reason for retry (e.g. "network_error", "server_error_5xx") */
  reason?: string;
}

/** W2.4: `chat:round-state` 事件 payload — token/轮次/耗时可观测性 */
export interface ChatRoundStatePayload {
  conversation_id: string;
  round: number;
  elapsed_ms: number;
  tokens_prompt: number;
  tokens_completion: number;
  cached_tokens: number;
  retry_count: number;
}

// ============================================================================
// P2-1 工具调用相关类型
// ============================================================================

/**
 * 消息内容块（与 Rust 侧 ContentBlock 对齐）
 *
 * P2-2：新增 `image` 变体用于多模态图片输入。
 * - `data`       base64 编码（**不含** `data:image/...;base64,` 前缀，前缀由 adapter 拼接）
 * - `media_type` MIME 类型，目前白名单：`image/png | image/jpeg | image/gif | image/webp`
 */
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; data: string; media_type: string }
  | { type: "tool_use"; id: string; name: string; input: string }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error?: boolean }
  | { type: "thinking"; thinking: string; signature?: string };

/** `chat:tool-call-start` 事件 payload */
export interface ChatToolCallStartPayload {
  conversation_id: string;
  message_id: string;
  id: string;
  name: string;
}

/** `chat:tool-call-delta` 事件 payload */
export interface ChatToolCallDeltaPayload {
  conversation_id: string;
  message_id: string;
  id: string;
  delta: string;
}

/** `chat:tool-call-end` 事件 payload */
export interface ChatToolCallEndPayload {
  conversation_id: string;
  message_id: string;
  id: string;
}

/** `chat:tool-result` 事件 payload */
export interface ChatToolResultPayload {
  conversation_id: string;
  message_id: string;
  tool_use_id: string;
  content: string;
  is_error: boolean;
}

/** `chat:thinking` 事件 payload */
export interface ChatThinkingPayload {
  conversation_id: string;
  message_id: string;
  content: string;
}

/**
 * `chat:summary-injected` 事件 payload (M1.5 A3-4 滚动摘要)
 *
 * 当 MemoryStage 触发摘要压缩后，Rust 侧 emit 此事件，
 * 前端在 ChatStatusBar 显示「已压缩 N 条」指示器。
 */
export interface ChatSummaryInjectedPayload {
  conversation_id: string;
  summary_tokens: number;
  original_count: number;
  kept_count: number;
}

// ============================================================================
// A2-3 工具授权事件 payload
// ============================================================================

/**
 * `chat:tool-auth-request` 事件 payload (Rust → Frontend)
 *
 * 工具执行遇到需要用户确认授权的场景时（例如路径不在白名单内），
 * Rust 侧 emit 此事件，前端弹窗后通过 `chat:tool-auth-response` 事件回传结果。
 *
 * - `request_id`  唯一 ID，与响应中的 `request_id` 对应
 * - `tool_use_id` LLM 端的工具调用 ID（用于工具结果回填）
 * - `tool_name`   工具名（人类友好映射）
 * - `file_path`   待访问的路径
 * - `arguments`   工具调用参数 JSON 字符串
 * - `reason`      触发原因（展示文案）
 */
export interface ToolAuthRequestPayload {
  request_id: string;
  tool_use_id: string;
  tool_name: string;
  file_path: string;
  arguments: string;
  conversation_id: string;
  message_id: string;
  reason: string;
}

/**
 * `chat:tool-auth-response` 事件 payload (Frontend → Rust)
 *
 * 用户在弹窗中点击「允许」或「拒绝」后由前端 emit，
 * Rust 侧用 `request_id` 匹配 oneshot 通道。
 */
export interface ToolAuthResponse {
  request_id: string;
  allowed: boolean;
}

/** 工具定义（前端发给后端的 tool schema） */
export interface ToolDef {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

// ============================================================================
// Template（用户自定义模板）
// ============================================================================
//
// 模板是「带变量占位符的 system prompt + user prompt 前缀」组合，
// 可在聊天中通过 @模板名 或芯片选择注入。详情见 icepaw-p0-p2-plan.md §2.4 P2-4。

/**
 * 模板变量定义。
 *
 * - name    变量名（占位符 {{name}} 替换目标）
 * - label   前端展示标签（中文友好名）
 * - type    控件类型：`text` | `textarea` | `select`
 * - default 默认值
 * - options 仅 `select` 类型有效
 */
export interface TemplateVariable {
  name: string;
  label: string;
  type: "text" | "textarea" | "select";
  default?: string | null;
  options?: string[] | null;
}

/**
 * 模板实体：对应数据库 `templates` 表中一行记录。
 *
 * - id                  主键
 * - name                模板名（用户可见，@ 触发时按 name 匹配）
 * - description         描述（列表展示用）
 * - system_prompt       system prompt 内容
 * - user_prompt_prefix  用户消息前缀（拼到用户消息前面）
 * - variables           变量定义列表
 * - tools               工具名列表（P2-1 落地后实际生效）
 * - sort_order          列表排序权重
 * - created_at/updated_at
 */
export interface Template {
  id: string;
  name: string;
  description: string;
  system_prompt: string;
  user_prompt_prefix: string;
  variables: TemplateVariable[];
  tools: string[];
  sort_order: number;
  created_at: string;
  updated_at: string;
}

/**
 * 创建模板入参。
 * 与 Template 差异：不含 id / 时间戳。
 */
export interface NewTemplate {
  name: string;
  description?: string;
  system_prompt?: string;
  user_prompt_prefix?: string;
  variables?: TemplateVariable[];
  tools?: string[];
  sort_order?: number;
}

/**
 * 更新模板入参（partial update）。
 * 字段语义：传了的字段会覆盖；不传的字段保持原值。
 */
export interface TemplateUpdate {
  id: string;
  name?: string;
  description?: string;
  system_prompt?: string;
  user_prompt_prefix?: string;
  variables?: TemplateVariable[];
  tools?: string[];
  sort_order?: number;
}

// ============================================================================
// UserPreferences（全局配置）
// ============================================================================

export interface UserPreferences {
  defaultAgentId?: string;
  defaultTemplateId?: string;
  onStartup?: string;
  language?: string;
  theme?: string;
  codeTheme?: string;
  fontSize?: number;
}
