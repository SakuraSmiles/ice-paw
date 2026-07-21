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
import type { Project, NewProject, ProjectMemberInput, ProjectPatch } from "../types";

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
        workspace_path: null,
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

  /**
   * 创建项目（推荐入口，自动写入初始 members）。
   *
   * @param input NewProject，包含 name/description/icon/workspace_path/agents
   */
  async function create(input: NewProject): Promise<Project> {
    const project = await bridge.projects.createWithAgents(input);
    projects.value.push(project);
    return project;
  }

  /**
   * 旧入口：部分更新项目（仅 name/description，向后兼容）。
   */
  async function update(
    id: string,
    name?: string,
    description?: string,
  ): Promise<Project> {
    const updated = await bridge.projects.update(id, name, description);
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx >= 0) projects.value[idx] = updated;
    return updated;
  }

  /**
   * 原子更新项目（字段 + 可选成员替换，推荐入口）。
   *
   * @param id      项目 ID
   * @param patch   ProjectPatch（字段缺失=不改，null=清空，string=覆盖）
   * @param members 可选成员列表（传了就整体替换，不传则不动）
   */
  async function updateFull(
    id: string,
    patch: ProjectPatch,
    members?: ProjectMemberInput[] | null,
  ): Promise<Project> {
    const updated = await bridge.projects.updateFull(id, patch, members);
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx >= 0) projects.value[idx] = updated;
    return updated;
  }

  /** 删除项目 */
  async function remove(id: string): Promise<void> {
    await bridge.projects.delete(id);
    projects.value = projects.value.filter((p) => p.id !== id);
    if (currentId.value === id) {
      setCurrent(DEFAULT_PROJECT_ID);
    }
  }

  /**
   * 编辑场景：整体替换项目成员。
   * 走单次 invoke，事务保证原子性；本地单点更新避免 loadAll 全量刷新。
   */
  async function setAgents(
    projectId: string,
    members: ProjectMemberInput[],
  ): Promise<void> {
    await bridge.projects.setAgents(projectId, members);
    const p = projects.value.find((p) => p.id === projectId);
    if (p) {
      p.agents = members.map((m) => ({
        agent_id: m.agent_id,
        role: m.role,
      }));
    }
  }

  /** 添加 Agent 到项目（细粒度入口，弹窗主流程不走） */
  async function addAgent(
    projectId: string,
    agentId: string,
    role: string = "member",
  ): Promise<void> {
    await bridge.projects.addAgent(projectId, agentId, role);
    // 本地单点更新，避免全量 loadAll 导致 UI 闪烁
    const proj = projects.value.find((p) => p.id === projectId);
    if (proj) {
      proj.agents.push({ agent_id: agentId, role });
    }
  }

  /** 从项目移除 Agent（细粒度入口） */
  async function removeAgent(projectId: string, agentId: string): Promise<void> {
    await bridge.projects.removeAgent(projectId, agentId);
    // 本地单点更新，避免全量 loadAll 导致 UI 闪烁
    const proj = projects.value.find((p) => p.id === projectId);
    if (proj) {
      proj.agents = proj.agents.filter((a) => a.agent_id !== agentId);
    }
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
    updateFull,
    remove,
    setAgents,
    addAgent,
    removeAgent,
    reorder,
  };
});
