<script setup lang="ts">
// 应用整体布局：左侧 Sidebar + 右侧内容区
//
// 启动流程：
//   1. onMounted → agentsStore.ensureLoaded()  加载 Agent 列表
//   2. agents 加载完成后 → Sidebar.vue 内部 onMounted 自动调 watchAgentChange()
//      加载当前 Agent 的会话列表
//   3. Sidebar emit chat:select → router.push 跳转到聊天页
//
// 无 Agent 时：右侧主区显示 EmptyAgentHint（覆盖 router-view）
// 有 Agent 时：正常显示 router-view

import { onMounted } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import Toast from "../common/Toast.vue";
import EmptyAgentHint from "../agent/EmptyAgentHint.vue";
import Sidebar from "./Sidebar.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const router = useRouter();

onMounted(async () => {
  // 1. 加载 Agent 列表
  await agentsStore.ensureLoaded();
  // 2. 若已有当前 Agent，主动加载会话列表（让 Sidebar 立即有数据可显示）
  if (agentsStore.currentId) {
    try {
      await conversationsStore.loadFor(agentsStore.currentId);
    } catch {
      // 加载失败由 AppLayout 通过 EmptyAgentHint 等 UI 兜底，此处不抛出
    }
  }
});

/** 从空状态点击「创建 Agent」跳转到管理页 */
function goToAgents(): void {
  void router.push({ name: "AgentManager" });
}

/**
 * Sidebar 选中会话：跳转到聊天页。
 * 当前 ChatPage 尚未实现会话加载，仅简单跳转。
 * 后续 chat 模块会监听 conversationsStore.currentId 自动加载消息。
 */
function onChatSelect(_conversationId: string | null): void {
  void router.push({ name: "Chat" });
}
</script>

<template>
  <div class="app-shell">
    <!-- 左侧：Sidebar（固定 260px） -->
    <aside class="app-sidebar">
      <Sidebar @chat:select="onChatSelect" />
    </aside>

    <!-- 右侧：主内容区 -->
    <main class="app-main">
      <router-view v-slot="{ Component }">
        <!-- 无 Agent 时：全局显示空状态提示 -->
        <EmptyAgentHint
          v-if="!agentsStore.hasAgents && !agentsStore.loading"
          @create="goToAgents"
        />
        <!-- 有 Agent 或加载中时：正常渲染路由页面 -->
        <component :is="Component" v-else />
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
  border-right: 1px solid var(--border, #e0e0e0);
  background: var(--sidebar-bg, #fafafa);
  height: 100%;
}

.app-main {
  flex: 1 1 auto;
  overflow: auto;
  min-width: 0;
  height: 100%;
}

@media (prefers-color-scheme: dark) {
  .app-sidebar {
    --border: #2a2a3a;
    --sidebar-bg: #1a1a2e;
  }
}
</style>