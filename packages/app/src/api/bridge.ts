// IcePaw 统一 invoke 入口
import { invoke } from "@tauri-apps/api/core";
import type {
  Agent,
  AgentUpdate,
  Conversation,
  Message,
  NewAgent,
  UserPreferences,
} from "../types";

function wrapInvokeError(op: string, err: unknown): Error {
  if (err instanceof Error) return new Error(`[bridge.${op}] ${err.message}`);
  if (typeof err === "object" && err !== null) {
    const obj = err as Record<string, unknown>;
    const msg = typeof obj.message === "string" ? obj.message : JSON.stringify(err);
    const kind = typeof obj.kind === "string" ? obj.kind : undefined;
    const prefix = kind ? `[${op}/${kind}]` : `[${op}]`;
    return new Error(`${prefix} ${msg}`);
  }
  return new Error(`[bridge.${op}] ${String(err)}`);
}

const agents = {
  async list(): Promise<Agent[]> {
    try { return await invoke<Agent[]>("list_agents"); }
    catch (err) { throw wrapInvokeError("agents.list", err); }
  },
  async create(input: NewAgent): Promise<Agent> {
    try { return await invoke<Agent>("create_agent", { input }); }
    catch (err) { throw wrapInvokeError("agents.create", err); }
  },
  async update(input: AgentUpdate): Promise<Agent> {
    try { return await invoke<Agent>("update_agent", { input }); }
    catch (err) { throw wrapInvokeError("agents.update", err); }
  },
  async rotateKey(agentId: string, apiKey: string, baseUrl?: string): Promise<void> {
    try { await invoke<void>("rotate_agent_api_key", { input: { agent_id: agentId, api_key: apiKey, base_url: baseUrl } }); }
    catch (err) { throw wrapInvokeError("agents.rotateKey", err); }
  },
  async delete(id: string): Promise<void> {
    try { await invoke<void>("delete_agent", { id }); }
    catch (err) { throw wrapInvokeError("agents.delete", err); }
  },
};

const conversations = {
  async listAll(): Promise<Conversation[]> {
    try { return await invoke<Conversation[]>("list_all_conversations"); }
    catch (err) { throw wrapInvokeError("conversations.listAll", err); }
  },
  async create(agentId: string, title?: string): Promise<Conversation> {
    try { return await invoke<Conversation>("create_conversation", { input: { agent_id: agentId, title } }); }
    catch (err) { throw wrapInvokeError("conversations.create", err); }
  },
  async rename(id: string, title: string): Promise<void> {
    try { await invoke<void>("rename_conversation", { id, title }); }
    catch (err) { throw wrapInvokeError("conversations.rename", err); }
  },
  async pin(id: string, pinned: boolean): Promise<void> {
    try { await invoke<void>("pin_conversation", { id, pinned }); }
    catch (err) { throw wrapInvokeError("conversations.pin", err); }
  },
  async delete(id: string): Promise<void> {
    try { await invoke<void>("delete_conversation", { id }); }
    catch (err) { throw wrapInvokeError("conversations.delete", err); }
  },
};

const messages = {
  async list(conversationId: string, opts?: { limit?: number; before?: [string, number] }): Promise<Message[]> {
    try { return await invoke<Message[]>("list_messages", { conversationId, ...opts }); }
    catch (err) { throw wrapInvokeError("messages.list", err); }
  },
};

const chat = {
  async sendMessage(conversationId: string, content: string): Promise<void> {
    try {
      await invoke<void>("send_message", {
        input: { conversation_id: conversationId, content, tools_enabled: false },
      });
    } catch (err) { throw wrapInvokeError("chat.sendMessage", err); }
  },
  async stopGeneration(conversationId: string): Promise<void> {
    try { await invoke<void>("stop_generation", { conversationId }); }
    catch (err) { throw wrapInvokeError("chat.stopGeneration", err); }
  },
};

const preferences = {
  async get(): Promise<UserPreferences> {
    try { return await invoke<UserPreferences>("get_preferences"); }
    catch (err) { throw wrapInvokeError("preferences.get", err); }
  },
  async set(key: string, value: unknown): Promise<void> {
    try { await invoke<void>("set_preference", { key, value: JSON.stringify(value) }); }
    catch (err) { throw wrapInvokeError("preferences.set", err); }
  },
};

export const bridge = { agents, conversations, messages, chat, preferences };
export default bridge;
