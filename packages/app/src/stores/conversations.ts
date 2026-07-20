// IcePaw 会话状态管理 Store
//
// 职责：
//   1. 维护当前项目的会话列表（Phase 2: 按项目分组）
//   2. 维护当前选中会话 ID（持久化到 localStorage）
//   3. 提供会话的 CRUD 与乐观更新（rename / pin / delete 失败时回滚）
//   4. 订阅 projectsStore.currentId 变化，自动加载对应项目的会话
//
// 设计要点：
//   - Composition API 风格（与 stores/agents.ts / stores/projects.ts 一致）
//   - Phase 2: 从 agent 维度切换到 project 维度
//   - 保留 loadFor(agentId) 向后兼容
//   - watchProjectChange() 由 Sidebar 在 projects 加载完成后显式调用一次
//
// 持久化：
//   - 当前会话 ID 持久化到 localStorage(`icepaw.lastConv.project.${projectId}`)

import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import { useAgentsStore } from "./agents";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "./projects";
import type { Conversation } from "../types";

// ============================================================================
// localStorage 工具
// ============================================================================

/** 生成「最近会话」持久化键名（项目维度） */
function lastConvKeyForProject(projectId: string): string {
  return `icepaw.lastConv.project.${projectId}`;
}

/** 读取指定项目的最近会话 ID */
function readLastConvForProject(projectId: string): string | null {
  try {
    return localStorage.getItem(lastConvKeyForProject(projectId));
  } catch {
    return null;
  }
}

/** 写入指定项目的最近会话 ID（null 表示清空） */
function writeLastConvForProject(projectId: string, id: string | null): void {
  try {
    if (id === null) {
      localStorage.removeItem(lastConvKeyForProject(projectId));
    } else {
      localStorage.setItem(lastConvKeyForProject(projectId), id);
    }
  } catch {
    // 忽略 localStorage 失败（隐私模式等）
  }
}

/** 生成「最近会话」持久化键名（Agent 维度，向后兼容） */
function lastConvKey(agentId: string): string {
  return `icepaw.lastConv.${agentId}`;
}

/** 读取指定 Agent 的最近会话 ID（向后兼容） */
function readLastConv(agentId: string): string | null {
  try {
    return localStorage.getItem(lastConvKey(agentId));
  } catch {
    return null;
  }
}

/** 写入指定 Agent 的最近会话 ID（向后兼容） */
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

export const useConversationsStore = defineStore("conversations", () => {
  // ============================================================================
  // state
  // ============================================================================

  /** 按 Agent ID 分组的会话列表（向后兼容，Phase 2 前使用） */
  const byAgent = ref<Record<string, Conversation[]>>({});

  /** 按项目 ID 分组的会话列表（Phase 2 新增） */
  const byProject = ref<Record<string, Conversation[]>>({});

  /** 当前选中的会话 ID */
  const currentId = ref<string | null>(null);

  /** 加载状态 */
  const loading = ref<boolean>(false);

  /** 正在重命名的会话 ID（null 表示无） */
  const renamingId = ref<string | null>(null);

  // ============================================================================
  // 内部工具 — Agent 维度（向后兼容）
  // ============================================================================

  function findInAgent(agentId: string, id: string): Conversation | null {
    const list = byAgent.value[agentId];
    if (!list) return null;
    return list.find((c) => c.id === id) ?? null;
  }

  // ============================================================================
  // 内部工具 — 项目维度（Phase 2）
  // ============================================================================

  function findInProject(projectId: string, id: string): Conversation | null {
    const list = byProject.value[projectId];
    if (!list) return null;
    return list.find((c) => c.id === id) ?? null;
  }

  function replaceInProject(projectId: string, updated: Conversation): void {
    const list = byProject.value[projectId];
    if (!list) return;
    const idx = list.findIndex((c) => c.id === updated.id);
    if (idx >= 0) {
      list.splice(idx, 1, updated);
    }
  }

  function resortProject(projectId: string): void {
    const list = byProject.value[projectId];
    if (!list) return;
    byProject.value[projectId] = sortConversations(list);
  }

  // ============================================================================
  // getters
  // ============================================================================

  /**
   * 当前会话实体（Phase 2: 依赖 projectsStore.currentId + currentId）。
   * 找不到时返回 null。
   */
  const current = computed<Conversation | null>(() => {
    const projectsStore = useProjectsStore();
    const projectId = projectsStore.currentId;
    if (!projectId || !currentId.value) return null;
    return findInProject(projectId, currentId.value);
  });

  // ------------------------------------------------------------------
  // Agent 维度 getters（向后兼容）
  // ------------------------------------------------------------------

  function listFor(agentId: string): Conversation[] {
    return sortConversations(byAgent.value[agentId] ?? []);
  }

  function pinned(agentId: string): Conversation[] {
    return listFor(agentId).filter((c) => c.pinned);
  }

  function unpinned(agentId: string): Conversation[] {
    return listFor(agentId).filter((c) => !c.pinned);
  }

  // ------------------------------------------------------------------
  // 项目维度 getters（Phase 2 新增）
  // ------------------------------------------------------------------

  /** 获取指定项目的全部会话（已排序） */
  function listForProject(projectId: string): Conversation[] {
    return sortConversations(byProject.value[projectId] ?? []);
  }

  /** 指定项目的已置顶会话 */
  function pinnedForProject(projectId: string): Conversation[] {
    return listForProject(projectId).filter((c) => c.pinned);
  }

  /** 指定项目的未置顶会话 */
  function unpinnedForProject(projectId: string): Conversation[] {
    return listForProject(projectId).filter((c) => !c.pinned);
  }

  // ============================================================================
  // actions — 加载
  // ============================================================================

  /**
   * 加载某项目的全部会话（Phase 2）。
   * - DEFAULT_PROJECT_ID 映射为 null（后端语义）
   * - 拉取后写入 byProject[projectId]
   * - 恢复上次 currentId
   *
   * @param projectId 项目 ID（DEFAULT_PROJECT_ID 或实际 UUID）
   */
  async function loadForProject(projectId: string): Promise<void> {
    loading.value = true;
    const actualId = projectId === DEFAULT_PROJECT_ID ? null : projectId;

    // 清空当前选中（新项目加载前）
    currentId.value = null;

    try {
      const list = await bridge.projects.listConversations(actualId);
      byProject.value[projectId] = sortConversations(list);

      // 恢复上次的 currentId
      const saved = readLastConvForProject(projectId);
      if (saved && list.some((c) => c.id === saved)) {
        currentId.value = saved;
      } else if (list.length > 0) {
        currentId.value = list[0]!.id;
        writeLastConvForProject(projectId, list[0]!.id);
      } else {
        currentId.value = null;
        writeLastConvForProject(projectId, null);
      }
    } catch (err) {
      byProject.value[projectId] = [];
      currentId.value = null;
      throw err;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 加载某 Agent 的全部会话（向后兼容）。
   * 保留原有行为，Phase 2 后不再由 UI 触发。
   */
  async function loadFor(agentId: string): Promise<void> {
    if (!agentId) return;
    loading.value = true;
    const prevId = currentId.value;
    const prevConv = prevId ? findInAgent(agentId, prevId) : null;
    if (!prevConv) {
      currentId.value = null;
    }
    try {
      const list = await bridge.conversations.list(agentId);
      byAgent.value[agentId] = sortConversations(list);

      const currentNow = currentId.value;
      if (currentNow && list.some((c) => c.id === currentNow)) {
        writeLastConv(agentId, currentNow);
        return;
      }

      const saved = readLastConv(agentId);
      if (saved && list.some((c) => c.id === saved)) {
        currentId.value = saved;
      } else if (list.length > 0) {
        currentId.value = list[0]!.id;
        writeLastConv(agentId, list[0]!.id);
      } else {
        currentId.value = null;
        writeLastConv(agentId, null);
      }
    } catch (err) {
      byAgent.value[agentId] = [];
      currentId.value = null;
      throw err;
    } finally {
      loading.value = false;
    }
  }

  // ============================================================================
  // actions — 选中
  // ============================================================================

  /**
   * 切换当前会话（Phase 2: 项目维度持久化）。
   * @param id 目标会话 ID；传 null 表示清空
   */
  function setCurrent(id: string | null): void {
    const projectsStore = useProjectsStore();
    const projectId = projectsStore.currentId;
    if (id !== null && projectId) {
      const list = byProject.value[projectId] ?? [];
      if (!list.some((c) => c.id === id)) {
        return;
      }
    }
    currentId.value = id;
    if (projectId) {
      writeLastConvForProject(projectId, id);
    }
  }

  // ============================================================================
  // actions — 创建
  // ============================================================================

  /**
   * 新建会话。成功后自动设为当前 + 持久化。
   * @param agentId 关联的 Agent ID
   * @param projectId 可选；所属项目 ID（不传则使用当前项目）
   * @returns 新创建的会话
   */
  async function create(
    agentId: string,
    projectId?: string | null,
  ): Promise<Conversation> {
    const projectsStore = useProjectsStore();
    const effectiveProjectId = projectId ?? projectsStore.currentId;
    const actualProjectId =
      effectiveProjectId === DEFAULT_PROJECT_ID ? null : effectiveProjectId;

    const created = await bridge.conversations.create(agentId, undefined, actualProjectId);

    // 写入项目维度缓存
    const storeKey = effectiveProjectId ?? DEFAULT_PROJECT_ID;
    const list = byProject.value[storeKey] ?? [];
    byProject.value[storeKey] = sortConversations([created, ...list]);
    currentId.value = created.id;
    writeLastConvForProject(storeKey, created.id);
    renamingId.value = null;
    return created;
  }

  // ============================================================================
  // actions — 重命名 / 置顶 / 删除（Phase 2: 项目维度）
  // ============================================================================

  async function rename(id: string, title: string): Promise<void> {
    const projectsStore = useProjectsStore();
    const projectId = projectsStore.currentId;
    if (!projectId) throw new Error("没有当前项目，无法重命名会话");

    const original = findInProject(projectId, id);
    if (!original) throw new Error("会话不存在");

    const trimmed = title.trim();
    if (trimmed.length === 0 || trimmed === original.title) {
      renamingId.value = null;
      return;
    }

    replaceInProject(projectId, { ...original, title: trimmed });
    try {
      await bridge.conversations.rename(id, trimmed);
      renamingId.value = null;
    } catch (err) {
      replaceInProject(projectId, original);
      renamingId.value = null;
      throw err;
    }
  }

  async function pin(id: string, pinnedValue: boolean): Promise<void> {
    const projectsStore = useProjectsStore();
    const projectId = projectsStore.currentId;
    if (!projectId) throw new Error("没有当前项目，无法置顶会话");

    const original = findInProject(projectId, id);
    if (!original) throw new Error("会话不存在");
    if (original.pinned === pinnedValue) return;

    replaceInProject(projectId, { ...original, pinned: pinnedValue });
    resortProject(projectId);
    try {
      await bridge.conversations.pin(id, pinnedValue);
    } catch (err) {
      replaceInProject(projectId, original);
      resortProject(projectId);
      throw err;
    }
  }

  async function deleteConv(id: string): Promise<string | null> {
    const projectsStore = useProjectsStore();
    const projectId = projectsStore.currentId;
    if (!projectId) throw new Error("没有当前项目，无法删除会话");

    const list = byProject.value[projectId] ?? [];
    const idx = list.findIndex((c) => c.id === id);
    if (idx < 0) throw new Error("会话不存在");

    const removed = list[idx]!;
    const newList = list.slice();
    newList.splice(idx, 1);
    const previousCurrent = currentId.value;

    byProject.value[projectId] = newList;
    if (currentId.value === id) {
      const next = newList[0]?.id ?? null;
      currentId.value = next;
      writeLastConvForProject(projectId, next);
    }
    if (renamingId.value === id) {
      renamingId.value = null;
    }

    try {
      await bridge.conversations.delete(id);
      return currentId.value;
    } catch (err) {
      const restored = newList.slice();
      restored.splice(idx, 0, removed);
      byProject.value[projectId] = sortConversations(restored);
      currentId.value = previousCurrent;
      writeLastConvForProject(projectId, previousCurrent);
      throw err;
    }
  }

  // ============================================================================
  // actions — 重命名态管理
  // ============================================================================

  function requestRename(id: string): void {
    renamingId.value = id;
  }

  function cancelRename(): void {
    renamingId.value = null;
  }

  // ============================================================================
  // 副作用：监听项目切换（Phase 2，仅注册一次）
  // ============================================================================

  /** watch 是否已注册 */
  let watchProjectRegistered = false;

  /** Agent watch 是否已注册（向后兼容） */
  let watchRegistered = false;

  /**
   * 订阅 projectsStore.currentId 变化：
   *   - 切换项目时重新 loadForProject + 重置 currentId
   *
   * 由 Sidebar 在 projectsStore.loadAll() 完成后调用一次。
   */
  function watchProjectChange(): void {
    if (watchProjectRegistered) return;
    watchProjectRegistered = true;

    const projectsStore = useProjectsStore();

    // 监听项目切换
    watch(
      () => projectsStore.currentId,
      (newId, oldId) => {
        if (newId === oldId) return;
        if (!newId) {
          currentId.value = null;
          renamingId.value = null;
          return;
        }
        void loadForProject(newId);
      },
    );
  }

  /**
   * 订阅 agentsStore.currentId 变化（向后兼容，Phase 2 前使用）。
   */
  function watchAgentChange(): void {
    if (watchRegistered) return;
    watchRegistered = true;

    const agentsStore = useAgentsStore();

    if (agentsStore.currentId && !byAgent.value[agentsStore.currentId]) {
      void loadFor(agentsStore.currentId);
    }

    watch(
      () => agentsStore.currentId,
      (newId, oldId) => {
        if (newId === oldId) return;
        if (!newId) {
          currentId.value = null;
          renamingId.value = null;
          return;
        }
        void loadFor(newId);
      },
    );
  }

  return {
    // state
    byAgent,
    byProject,
    currentId,
    loading,
    renamingId,
    // getters
    current,
    listFor,
    pinned,
    unpinned,
    listForProject,
    pinnedForProject,
    unpinnedForProject,
    // actions
    loadFor,
    loadForProject,
    setCurrent,
    create,
    rename,
    pin,
    delete: deleteConv,
    requestRename,
    cancelRename,
    watchAgentChange,
    watchProjectChange,
  };
});
