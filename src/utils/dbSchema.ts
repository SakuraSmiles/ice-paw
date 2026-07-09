// 数据库建表脚本：agents / conversations / messages
// 通过 initSchema() 在应用启动或首次进入测试页时调用。
// 使用 CREATE TABLE IF NOT EXISTS，幂等可重复执行。
//
// 重要：API Key 不存储在 SQLite 中（避免明文泄露），而是独立存放在
// 加密的 Keychain（tauri-plugin-store）中。agents 表只保存 provider 标识
// 用于关联到 Keychain 中的对应条目。

import database from "./database";

/**
 * agents 表：AI 智能体
 * - id: 主键
 * - name: 名称（必填）
 * - provider: 关联到 Keychain 中的 provider 标识（如 "openai" / "glm"），
 *             用于在运行时拉取对应的 API Key
 * - model: 使用的模型标识（如 gpt-4 / glm-4）
 * - system_prompt: 系统提示词
 * - created_at / updated_at: ISO 时间字符串，由 SQLite 默认生成
 */
export const CREATE_AGENTS_TABLE = `
CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  provider TEXT,
  model TEXT,
  system_prompt TEXT,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
`;

/**
 * conversations 表：会话
 * - id: 主键
 * - agent_id: 关联的智能体
 * - title: 会话标题
 */
export const CREATE_CONVERSATIONS_TABLE = `
CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  agent_id TEXT,
  title TEXT,
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (agent_id) REFERENCES agents(id)
);
`;

/**
 * messages 表：消息
 * - id: 主键
 * - conversation_id: 所属会话
 * - role: 角色（user / assistant / system）
 * - content: 消息内容
 */
export const CREATE_MESSAGES_TABLE = `
CREATE TABLE IF NOT EXISTS messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now')),
  FOREIGN KEY (conversation_id) REFERENCES conversations(id)
);
`;

/**
 * 执行所有建表语句（幂等）
 * 需要先确保数据库已通过 database.init() 完成连接。
 */
export async function initSchema(): Promise<void> {
  // 确保连接已建立
  await database.init();
  await database.execute(CREATE_AGENTS_TABLE);
  await database.execute(CREATE_CONVERSATIONS_TABLE);
  await database.execute(CREATE_MESSAGES_TABLE);
}

/**
 * agents 表行类型定义
 *
 * 注意：API Key 不在该类型中——需通过 Keychain.getKey(provider) 获取。
 */
export interface AgentRow {
  id: string;
  name: string;
  provider: string | null;
  model: string | null;
  system_prompt: string | null;
  created_at: string | null;
  updated_at: string | null;
}