<script setup lang="ts">
// App.vue — 应用根组件
// 初始化全局事件监听（流式聊天事件 + 键盘快捷键）
import { onMounted, onUnmounted } from "vue";
import { useChatStore } from "./stores/chat";
import { loadTimezone } from "./utils/time";

const chat = useChatStore();

function handleGlobalKeydown(e: KeyboardEvent) {
  const tag = (e.target as HTMLElement)?.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || (e.target as HTMLElement)?.isContentEditable) return;

  if (e.ctrlKey || e.metaKey) {
    switch (e.key.toLowerCase()) {
      case "n": e.preventDefault(); (document.querySelector(".conv-item-new") as HTMLButtonElement)?.click(); break;
      case "w": e.preventDefault(); chat.clearActiveConversation(); break;
      case "k": e.preventDefault(); (document.querySelector(".chat-textarea") as HTMLTextAreaElement)?.focus(); break;
    }
  }
}

onMounted(() => {
  useChatStore().initEvents();
  loadTimezone();
  document.addEventListener("keydown", handleGlobalKeydown);
});

onUnmounted(() => {
  document.removeEventListener("keydown", handleGlobalKeydown);
});
</script>

<template>
  <router-view />
</template>
