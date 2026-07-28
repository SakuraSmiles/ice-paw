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
  default_workspace_path?: string | null;
}
