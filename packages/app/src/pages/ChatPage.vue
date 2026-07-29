<script setup lang="ts">
// ChatPage.vue — 聊天对话视图（嵌入 AppLayout 右侧主区域）
import ChatHeader from "../components/chat/ChatHeader.vue";
import ChatMessages from "../components/chat/ChatMessages.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import ToolAuthDialog from "../components/chat/ToolAuthDialog.vue";
</script>

<template>
  <div class="chat-view">
    <ChatHeader />
    <ChatMessages />
    <ChatInput />
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
