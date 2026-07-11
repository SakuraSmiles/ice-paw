// Keychain 加密存储工具封装（兼容层）
//
// 历史背景：
//   早期版本基于 tauri-plugin-store 实现本地加密存储。
//   M2-B 之后底层迁移到 tauri-plugin-stronghold（由 Rust 侧 commands/agent_cmd.rs 接管），
//   前端统一通过 bridge.* 访问 vault，敏感字段（api_key 明文）不再触达浏览器侧。
//
// 本文件职责：
//   - 保留原有对外接口签名（KeychainEntry / KeychainEntryInput / KeychainError / saveKey
//     / getKey / deleteKey / listProviders / hasKey），让旧调用方零改动。
//   - 内部全部改为 invoke 薄封装；不再直接持有 Store 实例。
//
// 重要约定：
//   - provider 在本层等同于 agent_id（一一对应）。
//   - 新业务禁止直接使用 keychain.*，统一改走 src/api/bridge.ts 的 bridge.agents.*。
//   - getKey / deleteKey 已不再适用：api_key 仅由 Rust LLM 模块内部使用，前端无读取路径。

import { invoke } from "@tauri-apps/api/core";

/**
 * 单条密钥条目：用于描述某个 LLM Provider 的凭据信息。
 *
 * - provider   Provider 标识，等同于 agent_id（openai / glm / deepseek / anthropic ...）
 * - apiKey     该 Provider 的 API Key（明文保存到加密 store，不出现在日志/UI 列表中）
 * - baseUrl    自定义 API 地址（可选，例如代理或私有部署）
 * - createdAt  创建时间（ISO 字符串）
 * - updatedAt  最近更新时间（ISO 字符串）
 *
 * 注意：getKey 已被禁用，本结构当前仅作为类型兼容保留。
 */
export interface KeychainEntry {
  provider: string;
  apiKey: string;
  baseUrl?: string;
  createdAt: string;
  updatedAt: string;
}

/**
 * 保存 API Key 时使用的入参类型：调用方无需关心时间戳字段。
 */
export type KeychainEntryInput = Omit<KeychainEntry, "createdAt" | "updatedAt">;

/**
 * Keychain 错误：统一封装所有抛出的异常，便于上层捕获与诊断。
 * 错误信息面向开发者，包含操作上下文（例如 provider 名、底层错误）。
 */
export class KeychainError extends Error {
  constructor(
    message: string,
    public readonly cause?: unknown,
  ) {
    super(message);
    this.name = "KeychainError";
  }
}

// ============================================================================
// 内部工具
// ============================================================================

/**
 * 验证 provider 字符串合法性：去除前后空白后不能为空。
 * @throws {KeychainError} provider 为空时抛出
 */
function validateProvider(provider: string): string {
  const trimmed = provider.trim();
  if (trimmed.length === 0) {
    throw new KeychainError("provider 不能为空");
  }
  return trimmed;
}

/**
 * 将底层任意错误包装为 KeychainError，便于上层用 instanceof 统一捕获。
 */
function wrapError(op: string, err: unknown): KeychainError {
  const detail = err instanceof Error ? err.message : String(err);
  return new KeychainError(`Keychain ${op} 失败：${detail}`, err);
}

// ============================================================================
// KeychainManager
// ============================================================================

/**
 * Keychain 工具类：内部全部改为 invoke 薄封装。
 *
 * 整个应用通过 getKeychain() 共享同一实例，避免重复实例化。
 *
 * - saveKey()       调 rotate_agent_api_key 写入/更新 api_key（可选 base_url）
 * - getKey()        已禁用：api_key 明文仅由 Rust LLM 模块内部消费
 * - deleteKey()     已禁用：请改用 bridge.agents.delete 删除整个 Agent
 * - listProviders() 调 list_agents 返回所有 Agent 的 id（兼容旧语义）
 * - hasKey()        调 list_agents 检查某 id 是否存在
 */
class KeychainManager {
  /**
   * 保存（或覆盖）某 provider（即 agent_id）的 API Key。
   * 若 baseUrl 提供则同时更新。
   *
   * @param entry 待保存的条目（无需包含时间戳）
   *
   * 对应 Command：rotate_agent_api_key
   */
  async saveKey(entry: KeychainEntryInput): Promise<void> {
    const provider = validateProvider(entry.provider);
    try {
      await invoke<void>("rotate_agent_api_key", {
        agentId: provider,
        apiKey: entry.apiKey,
        baseUrl: entry.baseUrl,
      });
    } catch (err) {
      throw wrapError(`saveKey(provider=${provider})`, err);
    }
  }

  /**
   * 已禁用。
   *
   * 历史作用：按 provider 获取 api_key 明文。
   * 当前策略：api_key 明文仅由 Rust LLM 模块内部使用，前端业务层永远拿不到明文。
   * 调用方应改用 bridge.agents.list() 取得不含 key 的元信息。
   */
  async getKey(_provider: string): Promise<KeychainEntry | null> {
    console.warn("[keychain] getKey 已禁用：api_key 明文不再触达前端，请改用 bridge.agents.list()");
    return null;
  }

  /**
   * 已禁用。
   *
   * 历史作用：按 provider 删除单条 key 条目。
   * 当前策略：删除 key 等价于删除整个 Agent，应使用 bridge.agents.delete(id)。
   */
  async deleteKey(_provider: string): Promise<boolean> {
    console.warn("[keychain] deleteKey 已禁用：请改用 bridge.agents.delete(id) 删除整个 Agent");
    return false;
  }

  /**
   * 列出所有已配置的 provider（等价于所有 Agent 的 id）。
   * 对应 Command：list_agents
   */
  async listProviders(): Promise<string[]> {
    try {
      const agents = await invoke<Array<{ id: string }>>("list_agents");
      return agents.map((a) => a.id);
    } catch (err) {
      throw wrapError("listProviders", err);
    }
  }

  /**
   * 检查某 provider（即 agent_id）是否已配置。
   * 对应 Command：list_agents
   */
  async hasKey(provider: string): Promise<boolean> {
    const key = validateProvider(provider);
    try {
      const agents = await invoke<Array<{ id: string }>>("list_agents");
      return agents.some((a) => a.id === key);
    } catch (err) {
      throw wrapError(`hasKey(provider=${key})`, err);
    }
  }
}

// 单例：整个应用共享一个 Keychain 管理器
const keychain = new KeychainManager();

/** 获取全局共享的 Keychain 管理器 */
export function getKeychain(): KeychainManager {
  return keychain;
}

export default keychain;
