<script setup lang="ts">
// 应用整体布局（Phase 2 重构版）：左侧 Sidebar + 右侧内容区
//
// 启动流程（Phase 2）：
//   1. onMounted → agentsStore.ensureLoaded()  加载 Agent 列表
//   2. projectsStore.loadAll()  加载项目列表
//   3. conversationsStore.loadForProject()  加载当前项目的会话
//   4. Sidebar.vue 内部 onMounted 自动调 watchProjectChange()
//   5. Sidebar emit chat:select → router.push 跳转到项目聊天页
//
// 空状态说明：
//   - AppLayout 只负责「路由 + 全局 UI 壳」，不做业务拦截。
//   - 无 Agent 时的引导由各页面自行处理。

import { onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../../stores/projects";
import { useConversationsStore } from "../../stores/conversations";
import { useSettingsStore } from "../../stores/settings";
import Toast from "../common/Toast.vue";
import Sidebar from "./Sidebar.vue";

const agentsStore = useAgentsStore();
const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const settingsStore = useSettingsStore();
const router = useRouter();

/**
 * 全局「最近会话」localStorage 键名（P0-10：onStartup=last 使用）。
 *
 * 该 key 由 conversationsStore.setCurrent() / loadForProject() 写入；
 * AppLayout 在 onMounted 末尾读取并按 on_startup 设置决定是否跳转。
 */
const LAST_CONV_STORAGE_KEY = "icepaw.lastConvId";

/**
 * 将项目 ID 转换为路由参数值（默认项目映射为 "default"）。
 *
 * 与 ProjectManagerPage / Sidebar 保持一致。
 */
function projectIdToRouteParam(projectId: string): string {
  return projectId === DEFAULT_PROJECT_ID ? "default" : projectId;
}

onMounted(async () => {
  // 1. 加载 Agent 列表（后续仍需要 Agent 数据）
  await agentsStore.ensureLoaded();
  // 2. 加载项目列表
  try {
    await projectsStore.loadAll();
  } catch {
    // 加载失败不阻塞，使用默认项目
  }
  // 3. 加载当前项目的会话列表
  try {
    await conversationsStore.loadForProject(projectsStore.currentId || DEFAULT_PROJECT_ID);
  } catch {
    // 加载失败由各页面 UI 兜底
  }

  // 4. P0-10：消费 on_startup 设置
  //    - "last"：自动跳转到上次会话（按 icepaw.lastConvId）
  //    - "chat" / "none"：走默认重定向（router / → ProjectChat/default）
  try {
    await settingsStore.load();
  } catch {
    // 加载失败不影响启动，使用默认行为
    return;
  }

  const startup = settingsStore.prefs.on_startup ?? "chat";
  if (startup !== "last") return;

  const lastConvId = localStorage.getItem(LAST_CONV_STORAGE_KEY);
  const currentProjectId = projectsStore.currentId || DEFAULT_PROJECT_ID;
  if (!lastConvId || !currentProjectId) return;

  // 确认 lastConvId 属于当前项目（防止不同项目间的会话串扰）
  const exists = conversationsStore
    .listForProject(currentProjectId)
    .some((c) => c.id === lastConvId);
  if (!exists) return;

  await router.push({
    name: "ProjectChatConversation",
    params: {
      projectId: projectIdToRouteParam(currentProjectId),
      conversationId: lastConvId,
    },
  });
});

/**
 * Sidebar 选中会话：跳转到项目聊天页。
 *
 * Phase 2: 路由从 /chat 改为 /projects/:projectId/chat
 * - projectId 为 "default" 时映射 DEFAULT_PROJECT_ID
 */
function onChatSelect(_conversationId: string | null): void {
  const rawId = projectsStore.currentId || DEFAULT_PROJECT_ID;
  const projectId = projectIdToRouteParam(rawId);
  void router.push({ name: "ProjectChat", params: { projectId } });
}

function onGlobalKeydown(e: KeyboardEvent): void {
  if ((e.ctrlKey || e.metaKey) && e.key === ",") {
    e.preventDefault();
    void router.push("/settings/general");
  }
}

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeydown);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onGlobalKeydown);
});
</script>

<template>
  <div class="app-shell">
    <!-- 左侧：Sidebar（固定 260px） -->
    <aside class="app-sidebar">
      <Sidebar @chat:select="onChatSelect" />
    </aside>

    <!-- 右侧：主内容区：永远渲染 router-view，由各页面自行处理空状态 -->
    <main class="app-main">
      <router-view v-slot="{ Component }">
        <component :is="Component" />
      </router-view>
    </main>

    <!-- 全局 Toast 层 -->
    <Toast />
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.app-sidebar {
  width: 260px;
  flex-shrink: 0;
  border-right: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-primary);
  height: 100%;
}

.app-main {
  flex: 1 1 auto;
  overflow: auto;
  min-width: 0;
  height: 100%;
}
</style>
