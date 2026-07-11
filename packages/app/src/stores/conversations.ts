// IcePaw 会话状态管理 Store
//
// 职责：
//   1. 维护当前 Agent 的会话列表（按 agent_id 分组）
//   2. 维护当前选中会话 ID（持久化到 localStorage）
//   3. 提供会话的 CRUD 与乐观更新（rename / pin / delete 失败时回滚）
//   4. 订阅 agentsStore.currentId 变化，自动加载对应 Agent 的会话
//
// 设计要点：
//   - Composition API 风格（与 stores/agents.ts 一致）
//   - state/getters/actions 严格遵循 §2.2 接口契约
//   - 所有 invoke 通过 src/api/bridge.ts 的 bridge.conversations 命名空间
//   - 乐观更新：先改本地状态，再调 bridge，失败回滚到原始值
//   - watchAgentChange() 由 AppLayout 在 agents 加载完成后显式调用一次
//
// 持久化：
//   - 当前会话 ID 持久化到 localStorage(`icepaw.lastConv.${agentId}`)

import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import { useAgentsStore } from "./agents";
import type { Conversation } from "../types";

// ============================================================================
// localStorage 工具
// ============================================================================

/** 生成「最近会话」持久化键名 */
function lastConvKey(agentId: string): string {
  return `icepaw.lastConv.${agentId}`;
}

/** 读取指定 Agent 的最近会话 ID */
function readLastConv(agentId: string): string | null {
  try {
    return localStorage.getItem(lastConvKey(agentId));
  } catch {
    return null;
  }
}

/** 写入指定 Agent 的最近会话 ID（null 表示清空） */
function writeLastConv(agentId: string, id: string | null): void {
  try {
    if (id === null) {
      localStorage.removeItem(lastConvKey(agentId));
    } else {
      localStorage.setItem(lastConvKey(agentId), id);
    }
  } catch {
    // 忽略 localStorage 失败（隐私模式等）
  }
}

// ============================================================================
// 排序工具
// ============================================================================

/**
 * 会话列表排序：pinned DESC, updated_at DESC。
 * 输入任意 Conversation[]，返回新数组（不修改原数组）。
 */
function sortConversations(list: Conversation[]): Conversation[] {
  return [...list].sort((a, b) => {
    if (a.pinned !== b.pinned) {
      return a.pinned ? -1 : 1;
    }
    // ISO 8601 字符串可直接按字典序比较，等价于时间序
    if (a.updated_at < b.updated_at) return 1;
    if (a.updated_at > b.updated_at) return -1;
    return 0;
  });
}

// ============================================================================
// store
// ============================================================================

/**
 * 会话 Store
 *
 * state:
 *   - byAgent     按 Agent ID 分组的会话列表
 *   - currentId   当前选中的会话 ID
 *   - loading     加载状态
 *   - renamingId  处于重命名态的会话 ID（InlineRename 显示条件）
 *
 * getters:
 *   - current          当前会话实体
 *   - listFor(id)      指定 Agent 的全部会话（pinned DESC, updated_at DESC）
 *   - pinned(id)       已置顶会话
 *   - unpinned(id)     未置顶会话
 *
 * actions:
 *   - loadFor(agentId)         加载某 Agent 的会话列表 + 恢复当前选中
 *   - setCurrent(id|null)      切换当前会话（持久化）
 *   - create(agentId)          新建会话，自动设为当前
 *   - rename(id, title)        重命名（乐观更新 + 失败回滚）
 *   - pin(id, pinned)          置顶 / 取消置顶（乐观更新 + 失败回滚）
 *   - delete(id)               删除（乐观删除 + 失败回滚）
 *   - requestRename(id)        进入重命名态
 *   - cancelRename()           退出重命名态
 *   - watchAgentChange()       订阅 agentsStore.currentId 变化（AppLayout 调用）
 */
export const useConversationsStore = defineStore("conversations", () => {
  // ============================================================================
  // state
  // ============================================================================

  /** 按 Agent ID 分组的会话列表 */
  const byAgent = ref<Record<string, Conversation[]>>({});

  /** 当前选中的会话 ID（跨 Agent 共享同一个 currentId，切换 Agent 时会重置） */
  const currentId = ref<string | null>(null);

  /** 加载状态 */
  const loading = ref<boolean>(false);

  /** 正在重命名的会话 ID（null 表示无） */
  const renamingId = ref<string | null>(null);

  // ============================================================================
  // 内部工具
  // ============================================================================

  /**
   * 在 byAgent 中查找指定会话。
   * @returns Conversation 或 null
   */
  function findInAgent(agentId: string, id: string): Conversation | null {
    const list = byAgent.value[agentId];
    if (!list) return null;
    return list.find((c) => c.id === id) ?? null;
  }

  /**
   * 在 byAgent 中替换一条会话（按 ID 匹配）。
   * 未找到则静默忽略。
   */
  function replaceInAgent(agentId: string, updated: Conversation): void {
    const list = byAgent.value[agentId];
    if (!list) return;
    const idx = list.findIndex((c) => c.id === updated.id);
    if (idx >= 0) {
      list.splice(idx, 1, updated);
    }
  }

  /** 重新排序指定 Agent 的会话列表 */
  function resortAgent(agentId: string): void {
    const list = byAgent.value[agentId];
    if (!list) return;
    byAgent.value[agentId] = sortConversations(list);
  }

  // ============================================================================
  // getters
  // ============================================================================

  /**
   * 当前会话实体（依赖 agentsStore.currentId + currentId）。
   * 找不到时返回 null。
   */
  const current = computed<Conversation | null>(() => {
    const agentsStore = useAgentsStore();
    const agentId = agentsStore.currentId;
    if (!agentId || !currentId.value) return null;
    return findInAgent(agentId, currentId.value);
  });

  /**
   * 获取指定 Agent 的全部会话（已排序：pinned DESC, updated_at DESC）。
   */
  function listFor(agentId: string): Conversation[] {
    return sortConversations(byAgent.value[agentId] ?? []);
  }

  /** 已置顶的会话 */
  function pinned(agentId: string): Conversation[] {
    return listFor(agentId).filter((c) => c.pinned);
  }

  /** 未置顶的会话 */
  function unpinned(agentId: string): Conversation[] {
    return listFor(agentId).filter((c) => !c.pinned);
  }

  // ============================================================================
  // actions
  // ============================================================================

  /**
   * 加载某 Agent 的全部会话。
   * - 清空 currentId（避免指向已不存在的会话）
   * - 拉取后写入 byAgent[agentId]
   * - 自动恢复到 localStorage 中保存的 currentId（如果还存在），否则选第一个
   *
   * @param agentId 目标 Agent ID；空字符串/null 直接返回
   */
  async function loadFor(agentId: string): Promise<void> {
    if (!agentId) return;
    loading.value = true;
    currentId.value = null;
    try {
      const list = await bridge.conversations.list(agentId);
      byAgent.value[agentId] = sortConversations(list);

      // 恢复上次的 currentId
      const saved = readLastConv(agentId);
      if (saved && list.some((c) => c.id === saved)) {
        currentId.value = saved;
      } else if (list.length > 0) {
        currentId.value = list[0].id;
        writeLastConv(agentId, list[0].id);
      } else {
        writeLastConv(agentId, null);
      }
    } catch (err) {
      // 加载失败：写入空数组，保留 currentId 为 null
      byAgent.value[agentId] = [];
      throw err;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 切换当前会话。
   * @param id 目标会话 ID；传 null 表示清空
   * 同时持久化到 localStorage(`icepaw.lastConv.${agentId}`)
   */
  function setCurrent(id: string | null): void {
    const agentsStore = useAgentsStore();
    const agentId = agentsStore.currentId;
    // 校验：非 null 的 id 必须存在于 byAgent[currentAgentId]
    if (id !== null && agentId) {
      const list = byAgent.value[agentId] ?? [];
      if (!list.some((c) => c.id === id)) {
        // 非法 id：忽略（保护 localStorage 不被污染）
        return;
      }
    }
    currentId.value = id;
    if (agentId) {
      writeLastConv(agentId, id);
    }
  }

  /**
   * 新建会话。成功后自动设为当前 + 持久化。
   * @returns 新创建的会话
   */
  async function create(agentId: string): Promise<Conversation> {
    const created = await bridge.conversations.create(agentId);
    const list = byAgent.value[agentId] ?? [];
    byAgent.value[agentId] = sortConversations([created, ...list]);
    currentId.value = created.id;
    writeLastConv(agentId, created.id);
    // 退出重命名态（防止新会话继承旧态）
    renamingId.value = null;
    return created;
  }

  /**
   * 重命名会话（乐观更新 + 失败回滚）。
   * 空标题或与原标题相同视为 no-op。
   */
  async function rename(id: string, title: string): Promise<void> {
    const agentsStore = useAgentsStore();
    const agentId = agentsStore.currentId;
    if (!agentId) throw new Error("没有当前 Agent，无法重命名会话");

    const original = findInAgent(agentId, id);
    if (!original) throw new Error("会话不存在");

    const trimmed = title.trim();
    if (trimmed.length === 0 || trimmed === original.title) {
      renamingId.value = null;
      return;
    }

    // 乐观更新
    replaceInAgent(agentId, { ...original, title: trimmed });
    try {
      await bridge.conversations.rename(id, trimmed);
      renamingId.value = null;
    } catch (err) {
      // 回滚
      replaceInAgent(agentId, original);
      renamingId.value = null;
      throw err;
    }
  }

  /**
   * 置顶 / 取消置顶会话（乐观更新 + 失败回滚）。
   * 与当前值相同时为 no-op。
   */
  async function pin(id: string, pinnedValue: boolean): Promise<void> {
    const agentsStore = useAgentsStore();
    const agentId = agentsStore.currentId;
    if (!agentId) throw new Error("没有当前 Agent，无法置顶会话");

    const original = findInAgent(agentId, id);
    if (!original) throw new Error("会话不存在");

    if (original.pinned === pinnedValue) return;

    // 乐观更新 + 立即重排（保持 pinned 在前）
    replaceInAgent(agentId, { ...original, pinned: pinnedValue });
    resortAgent(agentId);
    try {
      await bridge.conversations.pin(id, pinnedValue);
    } catch (err) {
      // 回滚
      replaceInAgent(agentId, original);
      resortAgent(agentId);
      throw err;
    }
  }

  /**
   * 删除会话（乐观删除 + 失败回滚）。
   * 删除后：若被删的是当前会话，则切换到剩余的第一个会话（如果有）。
   *
   * @returns 切换后的 currentId（供 Sidebar 同步通知父组件）
   */
  async function deleteConv(id: string): Promise<string | null> {
    const agentsStore = useAgentsStore();
    const agentId = agentsStore.currentId;
    if (!agentId) throw new Error("没有当前 Agent，无法删除会话");

    const list = byAgent.value[agentId] ?? [];
    const idx = list.findIndex((c) => c.id === id);
    if (idx < 0) throw new Error("会话不存在");

    const removed = list[idx];
    const newList = list.slice();
    newList.splice(idx, 1);
    const previousCurrent = currentId.value;

    // 乐观删除
    byAgent.value[agentId] = newList;
    if (currentId.value === id) {
      const next = newList[0]?.id ?? null;
      currentId.value = next;
      writeLastConv(agentId, next);
    }
    if (renamingId.value === id) {
      renamingId.value = null;
    }

    try {
      await bridge.conversations.delete(id);
      return currentId.value;
    } catch (err) {
      // 回滚：恢复被删的会话到原位置
      const restored = newList.slice();
      restored.splice(idx, 0, removed);
      byAgent.value[agentId] = sortConversations(restored);
      currentId.value = previousCurrent;
      writeLastConv(agentId, previousCurrent);
      throw err;
    }
  }

  /** 进入重命名态 */
  function requestRename(id: string): void {
    renamingId.value = id;
  }

  /** 退出重命名态 */
  function cancelRename(): void {
    renamingId.value = null;
  }

  // ============================================================================
  // 副作用：监听 Agent 切换（仅注册一次）
  // ============================================================================

  /** watch 是否已注册（防止 AppLayout 多次调用导致重复监听） */
  let watchRegistered = false;

  /**
   * 订阅 agentsStore.currentId 变化：
   *   - 启动时若已有 currentId（来自 localStorage 恢复），触发一次 loadFor
   *   - 切换 Agent 时重新 loadFor + 重置 currentId
   *
   * 由 AppLayout 在 agentsStore.ensureLoaded() 完成后调用一次。
   */
  function watchAgentChange(): void {
    if (watchRegistered) return;
    watchRegistered = true;

    const agentsStore = useAgentsStore();

    // 立即检查一次：若当前已有 currentId 则主动加载
    if (agentsStore.currentId && !byAgent.value[agentsStore.currentId]) {
      void loadFor(agentsStore.currentId);
    }

    // 监听后续切换
    watch(
      () => agentsStore.currentId,
      (newId, oldId) => {
        if (newId === oldId) return;
        if (!newId) {
          // 没有当前 Agent：清空状态
          currentId.value = null;
          renamingId.value = null;
          return;
        }
        // 切换到新 Agent：重新加载会话列表
        void loadFor(newId);
      },
    );
  }

  return {
    // state
    byAgent,
    currentId,
    loading,
    renamingId,
    // getters
    current,
    listFor,
    pinned,
    unpinned,
    // actions
    loadFor,
    setCurrent,
    create,
    rename,
    pin,
    delete: deleteConv,
    requestRename,
    cancelRename,
    watchAgentChange,
  };
});