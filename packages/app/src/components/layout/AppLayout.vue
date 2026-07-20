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
import Toast from "../common/Toast.vue";
import Sidebar from "./Sidebar.vue";

const agentsStore = useAgentsStore();
const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const router = useRouter();

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
});

/**
 * Sidebar 选中会话：跳转到项目聊天页。
 *
 * Phase 2: 路由从 /chat 改为 /projects/:projectId/chat
 * - projectId 为 "default" 时映射 DEFAULT_PROJECT_ID
 */
function onChatSelect(_conversationId: string | null): void {
  const rawId = projectsStore.currentId || DEFAULT_PROJECT_ID;
  const projectId = rawId === DEFAULT_PROJECT_ID ? "default" : rawId;
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
