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
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  embedding_model?: string | null;
  description?: string;
  avatar?: string | null;
  workspace_path?: string | null;
  config_from_file?: boolean;
  created_at: string;
  updated_at: string;
  has_api_key: boolean;
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
  tool_trim_threshold?: number | null;
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  workspace_path?: string;
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
  tool_trim_threshold?: number | null;
  enabled_tools?: string[] | null;
  supports_vision?: boolean;
  workspace_path?: string | null;
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
  agent_ids?: string[];
}

/** 更新项目入参（partial update；undefined = 不改） */
export interface UpdateProject {
  id: string;
  name?: string;
  description?: string;
  icon?: string;
  workspace_path?: string | null;
  theme_color?: string | null;
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
}

/** 多轮工具调用中，每轮工具后创建下一轮 assistant 占位时 emit */
export interface ChatAssistantStartPayload {
  conversation_id: string;
  message_id: string;
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
  | { type: "thinking"; thinking: string; signature?: string };

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
export interface ToolAuthResponse {
  request_id: string; allowed: boolean;
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
}

// ============================================================================
// MCP Server
// ============================================================================

export type McpTrustLevel = "trusted" | "untrusted";

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
