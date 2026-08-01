<script setup lang="ts">
// ChatPage.vue — 聊天对话视图（嵌入 AppLayout 右侧主区域）
import ChatHeader from "../components/chat/ChatHeader.vue";
import ChatMessages from "../components/chat/ChatMessages.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import ChatWelcome from "../components/chat/ChatWelcome.vue";
import ToolAuthDialog from "../components/chat/ToolAuthDialog.vue";
import { useChatStore } from "../stores/chat";

// 无激活会话时显示欢迎态（替代误导性的空输入框）：有激活会话才渲染消息区+输入框
const chat = useChatStore();
</script>

<template>
  <div class="chat-view">
    <ChatHeader />
    <ChatWelcome v-if="!chat.activeConvId" />
    <template v-else>
      <ChatMessages />
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
</style>
