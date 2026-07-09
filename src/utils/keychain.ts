// Keychain 加密存储工具封装
// 基于 tauri-plugin-store 实现：底层使用 AES-256-GCM 加密，密钥绑定应用标识符。
// 用于安全保存用户的 LLM API Key 等敏感信息，避免在 SQLite 明文中存储。
//
// 注意：@tauri-apps/plugin-store 仅在 Tauri 原生窗口中可用，
// 在纯浏览器中调用会抛出错误，请使用 try-catch 包裹。

import { Store } from "@tauri-apps/plugin-store";

/**
 * 存储文件名：在 Tauri app_data_dir 下生成该文件，
 * 由插件自动加密落盘，用户无法直接读取明文。
 */
const STORE_FILE = "keychain.bin";

/**
 * 单条密钥条目：用于描述某个 LLM 提供方的凭据信息。
 *
 * - provider   提供方标识，例如 "openai" / "glm" / "deepseek" / "anthropic"
 * - apiKey     该提供方的 API Key（明文保存到加密 store，不出现在日志/UI 列表中）
 * - baseUrl    自定义 API 地址（可选，例如代理或私有部署）
 * - createdAt  创建时间（ISO 字符串）
 * - updatedAt  最近更新时间（ISO 字符串）
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
  constructor(message: string, public readonly cause?: unknown) {
    super(message);
    this.name = "KeychainError";
  }
}

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
 * 验证 apiKey 字符串合法性：去除前后空白后不能为空。
 * @throws {KeychainError} apiKey 为空时抛出
 */
function validateApiKey(apiKey: string): string {
  const trimmed = apiKey.trim();
  if (trimmed.length === 0) {
    throw new KeychainError("apiKey 不能为空");
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

/**
 * Keychain 工具类：基于 tauri-plugin-store 的加密存储。
 *
 * 整个应用通过 getKeychain() 共享同一实例，避免重复加载 store 文件。
 *
 * - saveKey()       保存（或更新）某 provider 的 API Key
 * - getKey()        按 provider 获取条目
 * - deleteKey()     按 provider 删除条目
 * - listProviders() 列出所有已保存的 provider
 * - hasKey()        检查某 provider 是否已配置
 */
class KeychainManager {
  private store: Store | null = null;

  /**
   * 获取 store 实例（懒加载单例）。
   * 首次调用会执行 Store.load(STORE_FILE)，后续直接复用内存中的实例。
   */
  private async getStore(): Promise<Store> {
    if (this.store) {
      return this.store;
    }
    try {
      this.store = await Store.load(STORE_FILE);
      return this.store;
    } catch (err) {
      throw wrapError("初始化 store", err);
    }
  }

  /**
   * 保存（或覆盖）某 provider 的 API Key。
   * 若该 provider 已有记录，则保留 createdAt 并刷新 updatedAt。
   *
   * @param entry 待保存的条目（无需包含时间戳）
   */
  async saveKey(entry: KeychainEntryInput): Promise<void> {
    const provider = validateProvider(entry.provider);
    const apiKey = validateApiKey(entry.apiKey);
    const baseUrl = entry.baseUrl?.trim() ?? "";
    try {
      const store = await this.getStore();
      const existing = await store.get<KeychainEntry>(provider);
      const now = new Date().toISOString();
      const next: KeychainEntry = {
        provider,
        apiKey,
        baseUrl: baseUrl.length > 0 ? baseUrl : undefined,
        createdAt: existing?.createdAt ?? now,
        updatedAt: now,
      };
      await store.set(provider, next);
      await store.save();
    } catch (err) {
      if (err instanceof KeychainError) {
        throw err;
      }
      throw wrapError(`saveKey(provider=${provider})`, err);
    }
  }

  /**
   * 按 provider 获取条目。若不存在则返回 null。
   */
  async getKey(provider: string): Promise<KeychainEntry | null> {
    const key = validateProvider(provider);
    try {
      const store = await this.getStore();
      const entry = await store.get<KeychainEntry>(key);
      return entry ?? null;
    } catch (err) {
      throw wrapError(`getKey(provider=${key})`, err);
    }
  }

  /**
   * 按 provider 删除条目，返回是否真的有记录被删除。
   */
  async deleteKey(provider: string): Promise<boolean> {
    const key = validateProvider(provider);
    try {
      const store = await this.getStore();
      const removed = await store.delete(key);
      if (removed) {
        await store.save();
      }
      return removed;
    } catch (err) {
      throw wrapError(`deleteKey(provider=${key})`, err);
    }
  }

  /**
   * 列出所有已保存的 provider 标识。
   * 注意：仅返回已配置的 key 名（不含 baseUrl 等元信息）。
   */
  async listProviders(): Promise<string[]> {
    try {
      const store = await this.getStore();
      return await store.keys();
    } catch (err) {
      throw wrapError("listProviders", err);
    }
  }

  /**
   * 检查某 provider 是否已配置。
   */
  async hasKey(provider: string): Promise<boolean> {
    const key = validateProvider(provider);
    try {
      const store = await this.getStore();
      return await store.has(key);
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