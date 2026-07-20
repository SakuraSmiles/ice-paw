// IcePaw 项目状态管理 Store（Phase 2）
//
// 职责：
//   1. 维护项目列表
//   2. 维护当前选中项目 ID（持久化到 localStorage）
//   3. 提供 CRUD 与关联管理 actions
//
// 设计要点：
//   - "默认项目" 用 '__default__' 常量 ID 表示（实际查询时映射为 null）
//   - 当前项目 ID 持久化到 localStorage('icepaw.currentProject')
//   - Composition API 风格，与 stores/agents.ts / stores/conversations.ts 保持一致

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import type { Project, NewProject } from "../types";

/** 默认项目的虚拟 ID（对应后端 project_id = NULL） */
export const DEFAULT_PROJECT_ID = "__default__";

const STORAGE_KEY = "icepaw.currentProject";

export const useProjectsStore = defineStore("projects", () => {
  // ========================================================================
  // State
  // ========================================================================

  const projects = ref<Project[]>([]);
  const currentId = ref<string>(DEFAULT_PROJECT_ID);
  const loading = ref(false);

  // ========================================================================
  // Getters
  // ========================================================================

  /** 当前项目（默认项目返回虚拟对象） */
  const current = computed<Project | null>(() => {
    if (currentId.value === DEFAULT_PROJECT_ID) {
      return {
        id: DEFAULT_PROJECT_ID,
        name: "默认项目",
        description: "",
        icon: "folder",
        sort_order: -1,
        created_at: "",
        updated_at: "",
        agents: [],
      };
    }
    return projects.value.find((p) => p.id === currentId.value) ?? null;
  });

  /** 排序后的项目列表（按 sort_order ASC） */
  const sortedProjects = computed(() => {
    return [...projects.value].sort((a, b) => a.sort_order - b.sort_order);
  });

  // ========================================================================
  // Actions
  // ========================================================================

  /** 加载全部项目 */
  async function loadAll(): Promise<void> {
    loading.value = true;
    try {
      projects.value = await bridge.projects.list();
    } finally {
      loading.value = false;
    }
  }

  /** 设置当前项目（持久化） */
  function setCurrent(id: string): void {
    currentId.value = id;
    try {
      localStorage.setItem(STORAGE_KEY, id);
    } catch {
      /* ignore */
    }
  }

  /** 创建项目 */
  async function create(input: NewProject): Promise<Project> {
    const project = await bridge.projects.create(input);
    projects.value.push(project);
    return project;
  }

  /** 更新项目 */
  async function update(
    id: string,
    name?: string,
    description?: string,
  ): Promise<void> {
    const updated = await bridge.projects.update(id, name, description);
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx >= 0) projects.value[idx] = updated;
  }

  /** 删除项目 */
  async function remove(id: string): Promise<void> {
    await bridge.projects.delete(id);
    projects.value = projects.value.filter((p) => p.id !== id);
    if (currentId.value === id) {
      setCurrent(DEFAULT_PROJECT_ID);
    }
  }

  /** 添加 Agent 到项目 */
  async function addAgent(
    projectId: string,
    agentId: string,
    role: string = "member",
  ): Promise<void> {
    await bridge.projects.addAgent(projectId, agentId, role);
    // 重新加载该项目
    await loadAll();
  }

  /** 从项目移除 Agent */
  async function removeAgent(projectId: string, agentId: string): Promise<void> {
    await bridge.projects.removeAgent(projectId, agentId);
    await loadAll();
  }

  /** 排序 */
  async function reorder(orderedIds: string[]): Promise<void> {
    await bridge.projects.reorder(orderedIds);
    await loadAll();
  }

  // ========================================================================
  // 初始化：从 localStorage 恢复当前项目
  // ========================================================================

  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) currentId.value = saved;
  } catch {
    /* ignore */
  }

  return {
    projects,
    currentId,
    loading,
    current,
    sortedProjects,
    loadAll,
    setCurrent,
    create,
    update,
    remove,
    addAgent,
    removeAgent,
    reorder,
  };
});
