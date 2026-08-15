<script setup lang="ts">
// ChatPage.vue — 聊天对话视图（嵌入 AppLayout 右侧主区域）
//
// 会话内标签页：标题下方常驻 [对话 | 轨迹]。发送消息是**会话级能力**，
// 输入框沉到标签页之下常驻——tab 只切换内容区的渲染形态（多 render 愿景）：
// - 对话/轨迹都是同一 session 的 render，任一 tab 都能发送；
// - 轨迹页发送 → 停留轨迹页：live 追加 + 生成中 ephemeral 行直接看进度；
// - v-show 双 pane：chat DOM 常驻 → 流式/滚动状态不因切 tab 中断；
// - 切换会话 → tab 重置回「对话」；无激活会话（欢迎态）→ 标签条隐藏。
import { ref, watch } from "vue";
import ChatHeader from "../components/chat/ChatHeader.vue";
import ChatMessages from "../components/chat/ChatMessages.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import ChatWelcome from "../components/chat/ChatWelcome.vue";
import ToolAuthDialog from "../components/chat/ToolAuthDialog.vue";
import TrajectoryView from "../components/trajectory/TrajectoryView.vue";
import { useChatStore } from "../stores/chat";

const chat = useChatStore();

type ChatTab = "chat" | "trajectory";
const activeTab = ref<ChatTab>("chat");

// 切会话 → 回到「对话」标签（轨迹视图自身会按新 conversationId 重载）
watch(() => chat.activeConvId, () => {
  activeTab.value = "chat";
});
</script>

<template>
  <div class="chat-view">
    <ChatHeader :has-tabbar="!!chat.activeConvId" />
    <ChatWelcome v-if="!chat.activeConvId" />
    <template v-else>
      <nav class="chat-tabbar">
        <button
          class="chat-tab"
          :class="{ active: activeTab === 'chat' }"
          @click="activeTab = 'chat'"
        >
          <span>对话</span>
        </button>
        <button
          class="chat-tab"
          :class="{ active: activeTab === 'trajectory' }"
          @click="activeTab = 'trajectory'"
        >
          <span>轨迹</span>
        </button>
      </nav>

      <!-- 内容区：tab 只切渲染形态，v-show 双 pane 保活 -->
      <div class="chat-render">
        <div v-show="activeTab === 'chat'" class="chat-pane">
          <ChatMessages />
        </div>
        <TrajectoryView
          v-show="activeTab === 'trajectory'"
          class="traj-pane"
          :conversation-id="chat.activeConvId!"
        />
      </div>

      <!-- 输入框：会话级能力，两个 render 共用，常驻底部 -->
      <ChatInput />
    </template>
    <ToolAuthDialog />
  </div>
</template>

<style scoped>
.chat-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
}

/* 标签条与 ChatHeader 视觉一体：同底色（明暗各自跟随 --color-chat-header-bg），
   无顶部分割线（header 侧配套去掉底边线，见 ChatHeader 的 has-tabbar 修饰） */
.chat-tabbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 24px;
  background: var(--color-chat-header-bg);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
}

.chat-tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 20px;
  border: none;
  background: none;
  cursor: pointer;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  border-bottom: 2px solid transparent;
  transition: color var(--ip-duration-fast) var(--ip-ease-out), border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.chat-tab:hover { color: var(--ip-color-text-secondary); }
.chat-tab.active { color: var(--ip-primary-600); border-bottom-color: var(--ip-primary-500); font-weight: var(--ip-font-weight-medium); }

/* 内容区（render 切换区）：tab 只决定这里渲染什么，输入框在其下常驻 */
.chat-render {
  flex: 1;
  min-height: 0;
  display: flex;
}

.chat-pane {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.traj-pane {
  flex: 1;
  min-width: 0;
}
</style>
