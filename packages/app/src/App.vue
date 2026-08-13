<script setup lang="ts">
// App.vue — 应用根组件
// 初始化全局事件监听（流式聊天事件 + 键盘快捷键）
import { onMounted, onUnmounted } from "vue";
import { useChatStore } from "./stores/chat";
import { useChatEvents } from "./composables/useChatEvents";
import { loadTimezone } from "./utils/time";

const chat = useChatStore();
// 事件监听拆卸函数（useChatEvents 返回；卸载时调用，补齐此前缺失的 teardown）
let cleanupChatEvents: (() => void) | null = null;

// 文件拖拽全局守卫：拖文件到「输入框以外」的区域时，阻止浏览器默认行为（导航 / 打开文件）。
// 真正的附件 intake 仍在 ChatInput 的 .input-wrapper 上（@drop→addAttachmentsFromFileList）；
// 这里只兜底，确保拖到消息列表 / 空白区不会把文件在 webview 里打开。
// （前提：tauri.conf.json 的 dragDropEnabled:false——否则原生窗口层会吞掉拖拽事件，
// webview 的 JS 根本收不到 dataTransfer.files，附件只能靠粘贴 / 选择按钮。）
function isFileDrag(e: DragEvent): boolean {
  return !!e.dataTransfer?.types?.includes("Files");
}
function handleGlobalDragOver(e: DragEvent) {
  if (isFileDrag(e)) e.preventDefault();
}
function handleGlobalDrop(e: DragEvent) {
  if (isFileDrag(e)) e.preventDefault();
}

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

onMounted(async () => {
  cleanupChatEvents = await useChatEvents();
  loadTimezone();
  document.addEventListener("keydown", handleGlobalKeydown);
  document.addEventListener("dragover", handleGlobalDragOver);
  document.addEventListener("drop", handleGlobalDrop);
});

onUnmounted(() => {
  cleanupChatEvents?.();
  document.removeEventListener("keydown", handleGlobalKeydown);
  document.removeEventListener("dragover", handleGlobalDragOver);
  document.removeEventListener("drop", handleGlobalDrop);
});
</script>

<template>
  <router-view />
</template>
