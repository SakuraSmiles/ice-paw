// 项目（Project）状态管理
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { Project, NewProject, UpdateProject } from "../types";
import { bridge } from "../api/bridge";

export const useProjectStore = defineStore("project", () => {
  const list = ref<Project[]>([]);
  const loading = ref(false);
  const loaded = ref(false);
  const activeProjectId = ref<string | null>(null);

  async function load(force = false) {
    if (loaded.value && !force) return;
    if (loading.value) return; // 已有进行中的加载，跳过避免并发覆盖
    loading.value = true;
    try {
      list.value = await bridge.projects.list();
      loaded.value = true;
      // 校验 activeProjectId：若指向已删除/归档的项目则清空，避免 app 处于"已选项目但找不到"的无效状态
      if (activeProjectId.value && !list.value.some((p) => p.id === activeProjectId.value)) {
        setActiveProject(null);
      }
    } catch (e) {
      console.error("加载项目列表失败:", e);
    } finally {
      loading.value = false;
    }
  }

  /** 设置当前活跃项目（校验存在性，无效 ID 会被拒绝并 warn）。
   *  列表未加载时乐观接受（load() 末尾会二次校验，无效则清空）。 */
  function setActiveProject(id: string | null): void {
    if (id === null) {
      activeProjectId.value = null;
      return;
    }
    if (!loaded.value) {
      // 列表未加载，乐观设置；load() 完成后会校验并清空无效 ID
      activeProjectId.value = id;
      return;
    }
    if (list.value.some((p) => p.id === id)) {
      activeProjectId.value = id;
    } else {
      console.warn(`setActiveProject: 项目 ${id} 不存在于列表中，已忽略`);
    }
  }

  const getById = (id: string) => list.value.find((p) => p.id === id) ?? null;
  const activeProject = computed(() =>
    activeProjectId.value ? getById(activeProjectId.value) : null,
  );
  /** 活跃项目（未归档）—— 切换器 / 管理页活跃列表用 */
  const activeProjects = computed(() => list.value.filter((p) => !p.archived));
  /** 已归档项目 —— 管理页归档区用 */
  const archivedProjects = computed(() => list.value.filter((p) => p.archived));

  async function create(input: NewProject): Promise<Project> {
    const created = await bridge.projects.create(input);
    // create 只返回 ProjectRow（无 agents），重新拉取以拿到成员关联
    await load(true);
    return created;
  }

  async function update(input: UpdateProject): Promise<void> {
    await bridge.projects.update(input);
    await load(true);
  }

  async function remove(id: string): Promise<void> {
    await bridge.projects.delete(id);
    if (activeProjectId.value === id) setActiveProject(null);
    await load(true);
  }

  async function reorder(ids: string[]): Promise<void> {
    await bridge.projects.reorder(ids);
    await load(true);
  }

  async function moveConversation(conversationId: string, projectId: string | null): Promise<void> {
    await bridge.projects.moveConversation(conversationId, projectId);
  }

  /** 归档项目（软删除）：从活跃列表收起，会话不动 */
  async function archive(id: string): Promise<void> {
    await bridge.projects.archive(id);
    if (activeProjectId.value === id) setActiveProject(null);
    await load(true);
  }
  /** 恢复归档项目 */
  async function unarchive(id: string): Promise<void> {
    await bridge.projects.unarchive(id);
    await load(true);
  }
  /** 永久删除：deleteConversations=true 连同会话删；false 会话转散落 */
  async function permanentDelete(id: string, deleteConversations: boolean): Promise<void> {
    await bridge.projects.permanentDelete(id, deleteConversations);
    if (activeProjectId.value === id) setActiveProject(null);
    await load(true);
  }

  return {
    list,
    loading,
    loaded,
    activeProjectId,
    setActiveProject,
    load,
    getById,
    activeProject,
    activeProjects,
    archivedProjects,
    create,
    update,
    remove,
    reorder,
    moveConversation,
    archive,
    unarchive,
    permanentDelete,
  };
});
