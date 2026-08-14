// IcePaw 统一 invoke 入口
import { invoke } from "@tauri-apps/api/core";
import type {
  Agent,
  AgentUpdate,
  Conversation,
  IndexStats,
  KbStats,
  RebuildStats,
  Kb,
  KbDocument,
  Message,
  McpServer,
  McpServerSnapshot,
  McpServerUpdate,
  McpToolDef,
  NewAgent,
  NewMcpServer,
  NewProject,
  Project,
  SessionEvent,
  UpdateProject,
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
  async create(agentId: string, title?: string, projectId?: string | null): Promise<Conversation> {
    try { return await invoke<Conversation>("create_conversation", { input: { agent_id: agentId, title, project_id: projectId ?? null } }); }
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

const projects = {
  async list(): Promise<Project[]> {
    try { return await invoke<Project[]>("list_projects"); }
    catch (err) { throw wrapInvokeError("projects.list", err); }
  },
  async create(input: NewProject): Promise<Project> {
    try { return await invoke<Project>("create_project", { input }); }
    catch (err) { throw wrapInvokeError("projects.create", err); }
  },
  async update(input: UpdateProject): Promise<Project> {
    try { return await invoke<Project>("update_project", { input }); }
    catch (err) { throw wrapInvokeError("projects.update", err); }
  },
  async delete(id: string): Promise<void> {
    try { await invoke<void>("delete_project", { id }); }
    catch (err) { throw wrapInvokeError("projects.delete", err); }
  },
  async reorder(ids: string[]): Promise<void> {
    try { await invoke<void>("reorder_projects", { ids }); }
    catch (err) { throw wrapInvokeError("projects.reorder", err); }
  },
  /** 全量替换成员；members 为 [agentId, role][]（对应后端 Vec<(String,String)>） */
  async setAgents(projectId: string, members: [string, string][]): Promise<void> {
    try { await invoke<void>("set_project_agents", { projectId, members }); }
    catch (err) { throw wrapInvokeError("projects.setAgents", err); }
  },
  async addAgent(projectId: string, agentId: string, role?: string): Promise<void> {
    try { await invoke<void>("add_project_agent", { projectId, agentId, role }); }
    catch (err) { throw wrapInvokeError("projects.addAgent", err); }
  },
  async removeAgent(projectId: string, agentId: string): Promise<void> {
    try { await invoke<void>("remove_project_agent", { projectId, agentId }); }
    catch (err) { throw wrapInvokeError("projects.removeAgent", err); }
  },
  /** 列出项目内会话；projectId=null → 散落会话 */
  async listConversations(projectId: string | null): Promise<Conversation[]> {
    try { return await invoke<Conversation[]>("list_conversations_by_project", { projectId }); }
    catch (err) { throw wrapInvokeError("projects.listConversations", err); }
  },
  async moveConversation(conversationId: string, projectId: string | null): Promise<void> {
    try { await invoke<void>("move_conversation_to_project", { conversationId, projectId }); }
    catch (err) { throw wrapInvokeError("projects.moveConversation", err); }
  },
  /** 归档项目（软删除，可恢复） */
  async archive(id: string): Promise<void> {
    try { await invoke<void>("archive_project", { id }); }
    catch (err) { throw wrapInvokeError("projects.archive", err); }
  },
  /** 恢复归档项目 */
  async unarchive(id: string): Promise<void> {
    try { await invoke<void>("unarchive_project", { id }); }
    catch (err) { throw wrapInvokeError("projects.unarchive", err); }
  },
  /** 永久删除：deleteConversations=true 连同会话一起删；false 会话转散落 */
  async permanentDelete(id: string, deleteConversations: boolean): Promise<void> {
    try { await invoke<void>("permanent_delete_project", { id, deleteConversations }); }
    catch (err) { throw wrapInvokeError("projects.permanentDelete", err); }
  },
};

const messages = {
  async list(conversationId: string, opts?: { limit?: number; before?: [string, number] }): Promise<Message[]> {
    try { return await invoke<Message[]>("list_messages", { conversationId, ...opts }); }
    catch (err) { throw wrapInvokeError("messages.list", err); }
  },
};

const chat = {
  async sendMessage(conversationId: string, content: string, contentBlocks?: import("../types").ContentBlock[], toolsEnabled?: boolean, files?: import("../types").AttachedFile[]): Promise<void> {
    try {
      await invoke<void>("send_message", {
        input: {
          conversation_id: conversationId,
          content: content || undefined,
          content_blocks: contentBlocks?.length ? contentBlocks : undefined,
          tools_enabled: toolsEnabled ?? true,
          // office/pdf 附件：后端在 send_message 入口 materialize 为 Text 块（doc::try_extract），
          // **不**进 content_blocks（文件是输入模态，非 ContentBlock）。
          files: files?.length ? files : undefined,
        },
      });
    } catch (err) { throw wrapInvokeError("chat.sendMessage", err); }
  },
  async stopGeneration(conversationId: string): Promise<void> {
    try { await invoke<void>("stop_generation", { conversationId }); }
    catch (err) { throw wrapInvokeError("chat.stopGeneration", err); }
  },
  /** 配置提案审批响应（invoke：原 emit 通道因 Tauri v2 事件作用域不匹配而失效） */
  async respondProposal(input: {
    request_id: string;
    decision: "approved" | "modified" | "rejected";
    reason?: string | null;
    changes?: Record<string, string>;
  }): Promise<void> {
    try { await invoke<void>("respond_config_proposal", { input }); }
    catch (err) { throw wrapInvokeError("chat.respondProposal", err); }
  },
  /** 工具授权响应（invoke：同上，双通道一起修复） */
  async respondAuth(input: {
    request_id: string;
    allowed: boolean;
    dont_ask_again?: boolean;
  }): Promise<void> {
    try { await invoke<void>("respond_tool_auth", { input }); }
    catch (err) { throw wrapInvokeError("chat.respondAuth", err); }
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

const mcp = {
  /** 列出所有 MCP Server 及其运行时状态 */
  async list(): Promise<McpServerSnapshot[]> {
    try { return await invoke<McpServerSnapshot[]>("list_mcp_servers"); }
    catch (err) { throw wrapInvokeError("mcp.list", err); }
  },
  async create(input: NewMcpServer): Promise<McpServer> {
    try { return await invoke<McpServer>("create_mcp_server", { input }); }
    catch (err) { throw wrapInvokeError("mcp.create", err); }
  },
  async update(input: McpServerUpdate): Promise<McpServer> {
    try { return await invoke<McpServer>("update_mcp_server", { input }); }
    catch (err) { throw wrapInvokeError("mcp.update", err); }
  },
  async remove(id: string): Promise<void> {
    try { await invoke<void>("delete_mcp_server", { id }); }
    catch (err) { throw wrapInvokeError("mcp.remove", err); }
  },
  /** 快速启用/禁用 */
  async setEnabled(id: string, enabled: boolean): Promise<void> {
    try { await invoke<void>("set_mcp_enabled", { id, enabled }); }
    catch (err) { throw wrapInvokeError("mcp.setEnabled", err); }
  },
  /** 重试失败的 server */
  async retry(id: string): Promise<McpToolDef[]> {
    try { return await invoke<McpToolDef[]>("retry_mcp_server", { id }); }
    catch (err) { throw wrapInvokeError("mcp.retry", err); }
  },
  /** 检测 Node.js 是否可用 */
  async checkNodejs(): Promise<boolean> {
    try { return await invoke<boolean>("check_nodejs"); }
    catch { return false; }
  },
  /** 列出内置工具清单（后端 register_builtin 单一来源，前端不再手抄） */
  async listBuiltinTools(): Promise<{ name: string; description: string }[]> {
    try { return await invoke<{ name: string; description: string }[]>("list_builtin_tools"); }
    catch (err) { throw wrapInvokeError("mcp.listBuiltinTools", err); }
  },
};

const kb = {
  async list(): Promise<Kb[]> {
    try { return await invoke<Kb[]>("list_kb"); }
    catch (err) { throw wrapInvokeError("kb.list", err); }
  },
  async listDocuments(kbId: string): Promise<KbDocument[]> {
    try { return await invoke<KbDocument[]>("list_kb_documents", { kbId }); }
    catch (err) { throw wrapInvokeError("kb.listDocuments", err); }
  },
  async reindex(kbId: string): Promise<IndexStats> {
    try { return await invoke<IndexStats>("reindex_kb", { id: kbId }); }
    catch (err) { throw wrapInvokeError("kb.reindex", err); }
  },
  async getStats(kbId: string): Promise<KbStats> {
    try { return await invoke<KbStats>("get_kb_stats", { kbId }); }
    catch (err) { throw wrapInvokeError("kb.getStats", err); }
  },
  async testEmbeddingConfig(provider: string, model: string, apiKey: string, baseUrl?: string): Promise<void> {
    try { await invoke<void>("test_embedding_config", { provider, model, apiKey, baseUrl: baseUrl ?? null }); }
    catch (err) { throw wrapInvokeError("kb.testEmbeddingConfig", err); }
  },
  async rebuildAllEmbeddings(): Promise<RebuildStats> {
    try { return await invoke<RebuildStats>("rebuild_all_embeddings"); }
    catch (err) { throw wrapInvokeError("kb.rebuildAllEmbeddings", err); }
  },
};

const logs = {
  /** tail 当前日志文件最近 lineCount 行（默认 500，上限 5000） */
  async get(lineCount?: number): Promise<string[]> {
    try { return await invoke<string[]>("get_logs", { lineCount }); }
    catch (err) { throw wrapInvokeError("logs.get", err); }
  },
  /** 返回 app 数据目录路径 */
  async getDataDir(): Promise<string> {
    try { return await invoke<string>("get_data_dir"); }
    catch (err) { throw wrapInvokeError("logs.getDataDir", err); }
  },
  /** 用文件管理器打开数据目录 */
  async openDataDir(): Promise<void> {
    try { await invoke<void>("open_data_dir"); }
    catch (err) { throw wrapInvokeError("logs.openDataDir", err); }
  },
};

const trajectory = {
  /** 读取会话事件流（seq 正序，payload 已 parse）；供「轨迹回放」视图消费。
   *  三形态：无参=全量 / limit+beforeSeq=尾部优先向前翻页 / limit+afterSeq=正向增量（live 追加轮询） */
  async listEvents(conversationId: string, limit?: number, beforeSeq?: number, afterSeq?: number): Promise<SessionEvent[]> {
    try {
      return await invoke<SessionEvent[]>("list_session_events", { conversationId, limit: limit ?? null, beforeSeq: beforeSeq ?? null, afterSeq: afterSeq ?? null });
    } catch (err) { throw wrapInvokeError("trajectory.listEvents", err); }
  },
  /** 导出会话轨迹为 JSONL 到下载目录；返回写入的文件绝对路径 */
  async exportJsonl(conversationId: string): Promise<string> {
    try {
      return await invoke<string>("export_session_trajectory", { conversationId });
    } catch (err) { throw wrapInvokeError("trajectory.exportJsonl", err); }
  },
};

export const bridge = { agents, conversations, projects, messages, chat, preferences, mcp, kb, logs, trajectory };
export default bridge;
