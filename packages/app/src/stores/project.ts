// 项目（Project）状态管理
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { Project, NewProject, UpdateProject, ProjectContext } from "../types";
import { bridge } from "../api/bridge";

export const useProjectStore = defineStore("project", () => {
  const list = ref<Project[]>([]);
  const loading = ref(false);
  const loaded = ref(false);
  const activeProjectId = ref<string | null>(null);

  /** 飞行中的加载请求——并发调用方共享同一个 Promise 而非跳过。
   *  （曾用「loading 中直接 return」防重复请求，但刷新时 Sidebar 的 load()
   *  在飞行中、详情页的 load(true) 立即返回空 → 误判「项目不存在」——
   *  直链组件必须能等飞行请求落地再下结论。） */
  let inflight: Promise<void> | null = null;

  async function load(force = false): Promise<void> {
    if (inflight) return inflight; // 等同一个请求（数据源相同，force 并发窗口内降级为等待）
    if (loaded.value && !force) return;
    loading.value = true;
    inflight = (async () => {
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
        inflight = null;
      }
    })();
    return inflight;
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

  // ===== 项目上下文（project.md / conventions.md）=====
  /** 最近一次读取的项目上下文（单条缓存：欢迎态状态条与项目页编辑区共用，
   *  避免两处各拉一次；切项目后 pid 对不上自然失效重拉） */
  const context = ref<(ProjectContext & { pid: string }) | null>(null);

  /** 读项目上下文；force=true 时绕过缓存（项目页编辑展开时用，防外部编辑器改后陈旧） */
  async function loadContext(pid: string, force = false): Promise<ProjectContext & { pid: string }> {
    if (!force && context.value?.pid === pid) return context.value;
    const out = await bridge.projects.getContext(pid);
    context.value = { pid, ...out };
    return context.value;
  }

  /** 写单个上下文文件并同步缓存（编辑区脏检查两文件分别保存） */
  async function saveContext(
    pid: string,
    file: "project.md" | "conventions.md",
    content: string,
  ): Promise<void> {
    await bridge.projects.setContext(pid, file, content);
    if (context.value?.pid === pid) {
      context.value =
        file === "project.md"
          ? { ...context.value, project_md: content }
          : { ...context.value, conventions_md: content };
    }
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
    context,
    loadContext,
    saveContext,
  };
});
