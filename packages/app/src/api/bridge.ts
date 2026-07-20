// IcePaw 统一 invoke 入口
//
// 设计目的：
//   业务代码不直接 `import { invoke } from "@tauri-apps/api/core"`，统一通过 bridge.* 调用。
//   好处：
//     1. 集中维护 Command 名称与参数映射，避免散落各处的魔法字符串
//     2. 类型签名单一来源，与 src/types/index.ts 严格对齐
//     3. 未来如需加日志/埋点/重试等横切能力，只改本文件
//
// Command 签名严格对齐清理方案 §2.3 / §2.4：
//   list_agents / create_agent / update_agent / rotate_agent_api_key / delete_agent
//   list_conversations / create_conversation / rename_conversation / pin_conversation / delete_conversation
//   list_messages / create_message

import { invoke } from "@tauri-apps/api/core";
import type {
  Agent,
  AgentUpdate,
  Conversation,
  Message,
  NewAgent,
  NewMessage,
  NewTemplate,
  Template,
  TemplateUpdate,
} from "../types";

// ============================================================================
// 内部工具：错误归一化
// ============================================================================

/**
 * 将任意 invoke 抛出的异常包装为 Error，便于上层用 message 统一处理。
 * 避免把 Tauri 原始的字符串/对象直接外泄到 UI。
 *
 * Tauri v2 invoke 失败时抛出的错误对象格式为 `{ kind, message }`（来自 Rust 侧的
 * AppError Serialize 实现）。如果直接 `String(err)` 会得到 "[object Object]"，
 * 因此这里按结构解析：error kind 进入前缀，message 作为正文。
 */
function wrapInvokeError(op: string, err: unknown): Error {
  if (err instanceof Error) {
    return new Error(`[bridge.${op}] ${err.message}`);
  }
  // Tauri 结构化错误对象：{ kind, message }
  if (typeof err === "object" && err !== null) {
    const obj = err as Record<string, unknown>;
    const msg = typeof obj.message === "string" ? obj.message : JSON.stringify(err);
    const kind = typeof obj.kind === "string" ? obj.kind : undefined;
    const prefix = kind ? `[${op}/${kind}]` : `[${op}]`;
    return new Error(`${prefix} ${msg}`);
  }
  return new Error(`[bridge.${op}] ${String(err)}`);
}

// ============================================================================
// bridge.agents — Agent 命名空间
// ============================================================================

const agents = {
  /**
   * 列出所有 Agent（不含 api_key 明文）。
   * 对应 Command：list_agents
   */
  async list(): Promise<Agent[]> {
    try {
      return await invoke<Agent[]>("list_agents");
    } catch (err) {
      throw wrapInvokeError("agents.list", err);
    }
  },

  /**
   * 创建 Agent。
   * 注意：api_key 仅在创建时通过此入口传入，Rust 侧会写入 stronghold vault。
   * 对应 Command：create_agent
   */
  async create(input: NewAgent): Promise<Agent> {
    try {
      return await invoke<Agent>("create_agent", { input });
    } catch (err) {
      throw wrapInvokeError("agents.create", err);
    }
  },

  /**
   * 部分更新 Agent（不含 api_key）。
   * 对应 Command：update_agent
   */
  async update(input: AgentUpdate): Promise<Agent> {
    try {
      return await invoke<Agent>("update_agent", { input });
    } catch (err) {
      throw wrapInvokeError("agents.update", err);
    }
  },

  /**
   * 轮换 Agent 的 api_key（可选同时更新 base_url）。
   * 注意：传空字符串视为「清空 key」。
   * 对应 Command：rotate_agent_api_key
   *
   * Rust 侧入参是结构体 `RotateAgentKey { agent_id, api_key, base_url }`
   * （无 `#[serde(rename_all)]`，因此需用 snake_case），所以这里走
   * `{ input: { agent_id, api_key, base_url } }` 包装形式（与 agents.create 同形）。
   *
   * @param agentId 目标 Agent ID
   * @param apiKey  新 api_key 明文（仅本次传输，Rust 侧会加密落 vault）
   * @param baseUrl 可选；同时更新 base_url
   */
  async rotateKey(agentId: string, apiKey: string, baseUrl?: string): Promise<void> {
    try {
      await invoke<void>("rotate_agent_api_key", {
        input: { agent_id: agentId, api_key: apiKey, base_url: baseUrl },
      });
    } catch (err) {
      throw wrapInvokeError("agents.rotateKey", err);
    }
  },

  /**
   * 删除 Agent。数据库 CASCADE 会自动清理关联的 conversations 与 messages。
   * 对应 Command：delete_agent
   */
  async delete(id: string): Promise<void> {
    try {
      await invoke<void>("delete_agent", { id });
    } catch (err) {
      throw wrapInvokeError("agents.delete", err);
    }
  },
};

// ============================================================================
// bridge.conversations — 会话命名空间
// ============================================================================

const conversations = {
  /**
   * 列出某 Agent 的全部会话。
   * Rust 侧按 `pinned DESC, updated_at DESC` 排序。
   * 对应 Command：list_conversations
   */
  async list(agentId: string): Promise<Conversation[]> {
    try {
      return await invoke<Conversation[]>("list_conversations", { agentId });
    } catch (err) {
      throw wrapInvokeError("conversations.list", err);
    }
  },

  /**
   * 新建会话。
   * title 可选；不传时由 Rust 侧默认空串，后续可由首条消息自动补标题。
   * 对应 Command：create_conversation
   *
   * Rust 侧入参是结构体 `NewConversation { agent_id, title }`
   * （无 `#[serde(rename_all)]`，因此需用 snake_case），所以这里走
   * `{ input: { agent_id, title } }` 包装形式（与 agents.create 同形）。
   */
  async create(agentId: string, title?: string): Promise<Conversation> {
    try {
      return await invoke<Conversation>("create_conversation", {
        input: { agent_id: agentId, title },
      });
    } catch (err) {
      throw wrapInvokeError("conversations.create", err);
    }
  },

  /**
   * 重命名会话。
   * 对应 Command：rename_conversation
   */
  async rename(id: string, title: string): Promise<void> {
    try {
      await invoke<void>("rename_conversation", { id, title });
    } catch (err) {
      throw wrapInvokeError("conversations.rename", err);
    }
  },

  /**
   * 置顶 / 取消置顶会话。
   * 对应 Command：pin_conversation
   */
  async pin(id: string, pinned: boolean): Promise<void> {
    try {
      await invoke<void>("pin_conversation", { id, pinned });
    } catch (err) {
      throw wrapInvokeError("conversations.pin", err);
    }
  },

  /**
   * 删除会话。数据库 CASCADE 会自动清理关联的 messages。
   * 对应 Command：delete_conversation
   */
  async delete(id: string): Promise<void> {
    try {
      await invoke<void>("delete_conversation", { id });
    } catch (err) {
      throw wrapInvokeError("conversations.delete", err);
    }
  },
};

// ============================================================================
// bridge.messages — 消息命名空间
// ============================================================================

const messages = {
  /**
   * 列出某会话的消息，支持分页。
   *
   * @param opts.limit   返回数量上限（默认由 Rust 侧决定，传 0/null 表示不限）
   * @param opts.before  复合游标 `[created_at, rowid]`，仅返回同时满足
   *                     `created_at < ts` 或 `created_at == ts && rowid < rowid`
   *                     的消息，用于向上翻页。
   *
   * 为什么不只用 `created_at`？SQLite 的 `datetime('now')` 是秒级精度，同一秒
   * 内的 user/assistant 对共享同一时间戳。纯字符串游标会让一次翻页恰好
   * 跳过整段同秒的历史（详见 icepaw-chat-perf-design.md §2.1）。
   *
   * 对应 Command：list_messages
   */
  async list(
    conversationId: string,
    opts?: { limit?: number; before?: [string, number] },
  ): Promise<Message[]> {
    try {
      return await invoke<Message[]>("list_messages", { conversationId, ...opts });
    } catch (err) {
      throw wrapInvokeError("messages.list", err);
    }
  },

  /**
   * 写入一条消息（用户消息或助手回复）。
   * 流式聊天中，助手消息由 Rust 侧在流结束后统一写入；前端一般只调此接口写用户消息。
   * 对应 Command：create_message
   */
  async create(input: NewMessage): Promise<Message> {
    try {
      return await invoke<Message>("create_message", { input });
    } catch (err) {
      throw wrapInvokeError("messages.create", err);
    }
  },
};

// ============================================================================
// bridge.chat — 流式聊天命名空间
// ============================================================================

const chat = {
  /**
   * 发送用户消息并触发 Rust 侧流式生成。
   * 命令本身立即返回（AppResult<()>），生成进度通过 `chat:start` / `chat:chunk`
   * / `chat:done` / `chat:error` 四个事件下发；前端在 stores/chat.ts 中订阅。
   *
   * 可选 `template` 参数（P2-4 模板注入）：传入后，Rust 侧会查模板 →
   * 渲染变量 → 替换/拼接 system_prompt，最后再调 LLM。
   *
   * P2-2 多模态：可选 `contentBlocks` 参数传入文字+图片等多模态块，
   * 与 `content` 互斥（传 `contentBlocks` 时 Rust 侧优先使用）。
   *
   * 注意：Rust 侧 send_message 的入参是结构体 SendMessageInput，因此这里走
   * `{ input: { conversation_id, content, template?, tools_enabled? } }` 包装形式。
   *
   * 对应 Command：send_message
   */
  async sendMessage(
    conversationId: string,
    content: string,
    template?: { template_id: string; values: Record<string, string> },
    toolsEnabled?: boolean,
    contentBlocks?: import("../types").ContentBlock[],
  ): Promise<void> {
    try {
      await invoke<void>("send_message", {
        input: {
          conversation_id: conversationId,
          content,
          template,
          tools_enabled: toolsEnabled ?? false,
          content_blocks: contentBlocks,
        },
      });
    } catch (err) {
      throw wrapInvokeError("chat.sendMessage", err);
    }
  },

  /**
   * 主动停止指定会话的流式生成（向 CancellationToken 派发 cancel 信号）。
   * 命令立即返回；真正的流中断由随后的 `chat:done` (finish_reason="abort") 或
   * `chat:error` 事件体现。
   *
   * 对应 Command：stop_generation
   */
  async stopGeneration(conversationId: string): Promise<void> {
    try {
      await invoke<void>("stop_generation", { conversationId });
    } catch (err) {
      throw wrapInvokeError("chat.stopGeneration", err);
    }
  },
};

// ============================================================================
// bridge.templates — 模板命名空间
// ============================================================================

const templates = {
  /**
   * 列出全部模板（按 sort_order ASC, created_at ASC）。
   * 对应 Command：list_templates
   */
  async list(): Promise<Template[]> {
    try {
      return await invoke<Template[]>("list_templates");
    } catch (err) {
      throw wrapInvokeError("templates.list", err);
    }
  },

  /**
   * 按 ID 取一条模板。
   * 对应 Command：get_template
   */
  async get(id: string): Promise<Template> {
    try {
      return await invoke<Template>("get_template", { id });
    } catch (err) {
      throw wrapInvokeError("templates.get", err);
    }
  },

  /**
   * 创建模板。
   * 对应 Command：create_template
   */
  async create(input: NewTemplate): Promise<Template> {
    try {
      return await invoke<Template>("create_template", { input });
    } catch (err) {
      throw wrapInvokeError("templates.create", err);
    }
  },

  /**
   * 部分更新模板。
   * 对应 Command：update_template
   */
  async update(input: TemplateUpdate): Promise<Template> {
    try {
      return await invoke<Template>("update_template", { input });
    } catch (err) {
      throw wrapInvokeError("templates.update", err);
    }
  },

  /**
   * 删除模板。
   * 对应 Command：delete_template
   */
  async delete(id: string): Promise<void> {
    try {
      await invoke<void>("delete_template", { id });
    } catch (err) {
      throw wrapInvokeError("templates.delete", err);
    }
  },
};


// ============================================================================
// bridge.preferences — 偏好设置命名空间
// ============================================================================

const preferences = {
  async get(): Promise<import("../types").UserPreferences> {
    try {
      return await invoke<import("../types").UserPreferences>("get_preferences");
    } catch (err) {
      throw wrapInvokeError("preferences.get", err);
    }
  },

  async set(key: string, value: unknown): Promise<void> {
    try {
      await invoke<void>("set_preference", {
        key,
        value: JSON.stringify(value),
      });
    } catch (err) {
      throw wrapInvokeError("preferences.set", err);
    }
  },
};
// ============================================================================
// 统一导出
// ============================================================================

/**
 * bridge 单一入口。
 *
 * 用法：
 * ```ts
 * import { bridge } from "@/api/bridge";
 * const agents = await bridge.agents.list();
 * ```
 *
 * 业务组件严禁直接 `invoke(...)`，所有调用必须经过本对象。
 */
export const bridge = {
  agents,
  conversations,
  messages,
  templates,
  chat,
  preferences,
};

export default bridge;
