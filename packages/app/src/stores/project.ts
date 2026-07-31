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
    loading.value = true;
    try {
      list.value = await bridge.projects.list();
      loaded.value = true;
    } catch (e) {
      console.error("加载项目列表失败:", e);
    } finally {
      loading.value = false;
    }
  }

  const getById = (id: string) => list.value.find((p) => p.id === id) ?? null;
  const activeProject = computed(() =>
    activeProjectId.value ? getById(activeProjectId.value) : null,
  );

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
    if (activeProjectId.value === id) activeProjectId.value = null;
    await load(true);
  }

  async function reorder(ids: string[]): Promise<void> {
    await bridge.projects.reorder(ids);
    await load(true);
  }

  async function moveConversation(conversationId: string, projectId: string | null): Promise<void> {
    await bridge.projects.moveConversation(conversationId, projectId);
  }

  return {
    list,
    loading,
    loaded,
    activeProjectId,
    load,
    getById,
    activeProject,
    create,
    update,
    remove,
    reorder,
    moveConversation,
  };
});
