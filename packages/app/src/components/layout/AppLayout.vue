<script setup lang="ts">
// 应用整体布局：左侧 Sidebar + 右侧内容区
//
// 启动流程：
//   1. onMounted → agentsStore.ensureLoaded()  加载 Agent 列表
//   2. agents 加载完成后 → Sidebar.vue 内部 onMounted 自动调 watchAgentChange()
//      加载当前 Agent 的会话列表
//   3. Sidebar emit chat:select → router.push 跳转到聊天页
//
// 空状态说明：
//   - AppLayout 只负责「路由 + 全局 UI 壳」，不做业务拦截。
//   - 无 Agent 时的引导由各页面自行处理：
//       • AgentManagerPage → 自己的 EmptyAgentHint（创建入口）
//       • ChatPage          → 区分「无 Agent」与「无会话」两种空态
//   - 历史版本在 AppLayout 中覆盖 EmptyAgentHint 会导致 router-view 被替换，
//     即使点击「创建 Agent」也无法真正跳转到 AgentManagerPage（死锁）。
//     已在本文件移除该全局拦截。

import { onMounted } from "vue";
import { useRouter } from "vue-router";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import Toast from "../common/Toast.vue";
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
      // 加载失败由各页面 UI 兜底，此处不抛出
    }
  }
});

/**
 * Sidebar 选中会话：跳转到聊天页。
 *
 * - Sidebar 的 onSelect / onCreate / onDelete 均会 emit chat:select；
 * - Sidebar 内部已调 conversationsStore.setCurrent / create，本组件只需负责路由跳转。
 * - ChatPage 监听 conversationsStore.currentId 自动 loadMessages，无需在此处传参。
 * - 若已在 /chat，push 到同名路由是 no-op（Vue Router 会忽略），无需额外判重。
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