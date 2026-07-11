// IcePaw Agent 状态管理 Store
//
// 职责：
//   1. 维护前端可用的 Agent 列表（不含 api_key 明文）
//   2. 维护当前选中 Agent（持久化到 localStorage）
//   3. 提供 CRUD 操作的 actions，封装 bridge.agents.* 调用与本地状态同步
//
// 设计要点：
//   - Composition API 风格（与 stores/ui.ts 一致）
//   - state/getters/actions 严格遵循 §1.2 接口契约
//   - 所有 invoke 通过 src/api/bridge.ts 的 bridge.agents 命名空间，禁止直接调用 Tauri invoke
//   - 失败状态写入 error 字段，由上层 Toast 提示
//
// 持久化：
//   - currentId 持久化到 localStorage('icepaw.lastAgent')
//   - 已加载标记也持久化（避免冷启动重复拉取）

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import type { Agent, AgentUpdate, NewAgent } from "../types";

// localStorage 键名
const LS_LAST_AGENT = "icepaw.lastAgent";
const LS_LOADED_FLAG = "icepaw.agentsLoaded";

/**
 * Agent Store
 *
 * state:
 *   - agents       Agent 列表（不含 api_key）
 *   - currentId    当前选中 Agent 的 ID
 *   - loading      是否处于加载中
 *   - error        最近的错误信息（字符串，供 Toast 显示）
 *   - loaded       是否已经成功加载过（用于 ensureLoaded 幂等判断）
 *
 * getters:
 *   - current      当前 Agent 实体（currentId 为空时为 null）
 *   - hasAgents    是否存在 Agent
 *   - byId(id)     按 ID 查找 Agent
 *
 * actions:
 *   - ensureLoaded()        幂等加载：未加载过时拉取一次
 *   - fetchAll()            强制刷新列表
 *   - setCurrent(id)        切换当前 Agent（持久化）
 *   - createOne(input)      创建 Agent，成功后写入列表 + 设为 current
 *   - updateOne(id, patch)  更新 Agent，成功后替换列表中的实体
 *   - deleteOne(id)         删除 Agent，成功后从列表移除 + 若为 current 则清空
 *   - rotateKey(...)        轮换 api_key（不动列表，错误走 error）
 */
export const useAgentsStore = defineStore("agents", () => {
  // ============================================================================
  // state
  // ============================================================================

  /** Agent 列表 */
  const agents = ref<Agent[]>([]);

  /** 当前选中 Agent 的 ID */
  const currentId = ref<string | null>(null);

  /** 列表加载状态 */
  const loading = ref<boolean>(false);

  /** 最近一次错误信息（供 Toast 显示；调用方读完建议清空） */
  const error = ref<string | null>(null);

  /** 是否已成功加载过（持久化标记，避免冷启动重复拉取） */
  const loaded = ref<boolean>(false);

  // ============================================================================
  // getters
  // ============================================================================

  /** 当前选中的 Agent 实体（找不到时为 null） */
  const current = computed<Agent | null>(() => {
    if (!currentId.value) return null;
    return agents.value.find((a) => a.id === currentId.value) ?? null;
  });

  /** 是否存在 Agent */
  const hasAgents = computed<boolean>(() => agents.value.length > 0);

  /**
   * 按 ID 查找 Agent
   * @param id Agent ID
   * @returns 找到的 Agent 或 undefined
   */
  function byId(id: string): Agent | undefined {
    return agents.value.find((a) => a.id === id);
  }

  // ============================================================================
  // actions
  // ============================================================================

  /**
   * 幂等加载：首次进入 App 时拉取一次。
   * - 已加载则直接返回
   * - 未加载则调 fetchAll，成功后置 loaded=true 并写 localStorage
   * - 失败：写入 error，loaded 保持 false（下次可重试）
   */
  async function ensureLoaded(): Promise<void> {
    if (loaded.value) return;
    // 同步 localStorage 标记，避免多窗口重复拉取
    try {
      if (localStorage.getItem(LS_LOADED_FLAG) === "1") {
        loaded.value = true;
      }
      if (loaded.value) {
        // 已经标记为已加载，仍然从后端拉一次以保证数据最新
        await fetchAll();
        // 恢复上次的 currentId
        const saved = localStorage.getItem(LS_LAST_AGENT);
        if (saved && agents.value.some((a) => a.id === saved)) {
          currentId.value = saved;
        } else if (agents.value.length > 0) {
          currentId.value = agents.value[0].id;
        }
        return;
      }
    } catch {
      // localStorage 不可用（隐私模式）时忽略
    }

    await fetchAll();

    try {
      localStorage.setItem(LS_LOADED_FLAG, "1");
      loaded.value = true;
    } catch {
      // 忽略
    }

    // 恢复上次的 currentId
    let saved: string | null = null;
    try {
      saved = localStorage.getItem(LS_LAST_AGENT);
    } catch {
      saved = null;
    }
    if (saved && agents.value.some((a) => a.id === saved)) {
      currentId.value = saved;
    } else if (agents.value.length > 0) {
      currentId.value = agents.value[0].id;
    }
  }

  /**
   * 强制刷新列表。
   * - 拉取过程中 loading=true
   * - 失败：写入 error，保留旧列表
   */
  async function fetchAll(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      const list = await bridge.agents.list();
      agents.value = list;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 切换当前 Agent。
   * @param id 目标 Agent 的 ID；传 null 表示清空当前选中
   * 同时持久化到 localStorage('icepaw.lastAgent')
   */
  function setCurrent(id: string | null): void {
    if (id !== null && !agents.value.some((a) => a.id === id)) {
      // 非法 ID：不更新 currentId，但允许 setCurrent(null) 清空
      return;
    }
    currentId.value = id;
    try {
      if (id === null) {
        localStorage.removeItem(LS_LAST_AGENT);
      } else {
        localStorage.setItem(LS_LAST_AGENT, id);
      }
    } catch {
      // 忽略 localStorage 失败
    }
  }

  /**
   * 创建 Agent。
   * 成功后：插入到列表头部 + 设为 current。
   * @returns 新创建的 Agent
   */
  async function createOne(input: NewAgent): Promise<Agent> {
    error.value = null;
    try {
      const created = await bridge.agents.create(input);
      agents.value = [created, ...agents.value];
      currentId.value = created.id;
      try {
        localStorage.setItem(LS_LAST_AGENT, created.id);
      } catch {
        // 忽略
      }
      return created;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  /**
   * 更新 Agent（不含 api_key）。
   * 成功后：替换列表中对应实体。
   * @returns 更新后的 Agent
   */
  async function updateOne(id: string, patch: AgentUpdate): Promise<Agent> {
    error.value = null;
    try {
      const updated = await bridge.agents.update({ ...patch, id });
      const idx = agents.value.findIndex((a) => a.id === id);
      if (idx >= 0) {
        agents.value.splice(idx, 1, updated);
      } else {
        agents.value.push(updated);
      }
      return updated;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  /**
   * 删除 Agent。
   * 成功后：从列表移除；若为 current 则清空 currentId。
   * 数据库 CASCADE 会清理关联的 conversations/messages（由 Rust 侧保证）。
   */
  async function deleteOne(id: string): Promise<void> {
    error.value = null;
    try {
      await bridge.agents.delete(id);
      const idx = agents.value.findIndex((a) => a.id === id);
      if (idx >= 0) {
        agents.value.splice(idx, 1);
      }
      if (currentId.value === id) {
        currentId.value = null;
        try {
          localStorage.removeItem(LS_LAST_AGENT);
        } catch {
          // 忽略
        }
      }
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  /**
   * 轮换 Agent 的 api_key（可选同时更新 base_url）。
   * 不动列表内容；失败走 error 字段。
   *
   * @param agentId 目标 Agent ID
   * @param apiKey  新 api_key 明文（仅本次传输，Rust 侧加密入 vault）
   * @param baseUrl 可选；同时更新 base_url
   */
  async function rotateKey(agentId: string, apiKey: string, baseUrl?: string): Promise<void> {
    error.value = null;
    try {
      await bridge.agents.rotateKey(agentId, apiKey, baseUrl);
      // base_url 更新可能影响 Agent 实体，刷新一下
      if (baseUrl !== undefined) {
        await fetchAll();
      }
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  return {
    // state
    agents,
    currentId,
    loading,
    error,
    loaded,
    // getters
    current,
    hasAgents,
    byId,
    // actions
    ensureLoaded,
    fetchAll,
    setCurrent,
    createOne,
    updateOne,
    deleteOne,
    rotateKey,
  };
});