<script setup lang="ts">
// App.vue — 应用根组件
// 初始化全局事件监听（流式聊天事件 + 键盘快捷键）
import { onMounted, onUnmounted, watch } from "vue";
import { useRouter } from "vue-router";
import { useChatStore } from "./stores/chat";
import { useProjectStore } from "./stores/project";
import { useScreenChannelStore } from "./stores/screenChannel";
import { useChatEvents } from "./composables/useChatEvents";
import { loadTimezone } from "./utils/time";
import { saveLastSession } from "./utils/sessionRestore";

const chat = useChatStore();
const project = useProjectStore();
const screenChannel = useScreenChannelStore();
const router = useRouter();

// 批次④ 步骤 2：/screen-hud、/screen-frame 是独立工具窗（非主窗）。localStorage
// 同源共享——工具窗若参与主窗的会话记忆写入（saveLastSession watch）会把
// {route:"/screen-hud"} 覆盖进主窗的启动恢复态；聊天事件接线/全局快捷键对
// 无聊天 UI 的工具窗也无意义。工具窗只保留 screenChannel 事件接线 + 拖拽守卫。
const isToolWindow =
  window.location.pathname.startsWith("/screen-hud") ||
  window.location.pathname.startsWith("/screen-frame");

// 启动恢复的记忆写入：路由 / 活跃会话 / 侧栏 scope 任一变化即落盘——每次
// 导航写一次（廉价），崩溃/断电不丢，不依赖窗口关闭事件。恢复侧在 Sidebar
// onMounted（见 planRestore 决策纯函数），本 watch 不读只写、无循环风险。
// ⚠️ 必须是「多源独立 getter」而非单 getter 返回数组：后者每次求值产新数组
// 引用，vue-router 初始导航 currentRoute 引用更换（值仍 '/'）即误触发，
// 在 Sidebar 恢复前把空状态 {convId:null} 写进 localStorage 覆盖上次记忆
// ——真机「重启永远欢迎态」的根因。多源逐元素 Object.is，值不变不写。
watch(
  [
    () => router.currentRoute.value.fullPath,
    () => chat.activeConvId,
    () => project.activeProjectId,
  ],
  ([route, convId, projectId]) => {
    if (isToolWindow) return;
    saveLastSession({ route, convId, projectId });
  },
);
// 事件监听拆卸函数（useChatEvents / screenChannel.init 返回；卸载时调用，补齐此前缺失的 teardown）
let cleanupChatEvents: (() => void) | null = null;
let cleanupScreenChannel: (() => void) | null = null;

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
  if (!isToolWindow) {
    cleanupChatEvents = await useChatEvents();
    loadTimezone();
    document.addEventListener("keydown", handleGlobalKeydown);
  }
  // 屏幕共享通道（批次④）：初拉通道态 + 订阅全量事件（幂等；进程级单例，重启即 Off）。
  // 工具窗（HUD）也要接线——它就是这些事件的消费者。
  cleanupScreenChannel = await screenChannel.init();
  document.addEventListener("dragover", handleGlobalDragOver);
  document.addEventListener("drop", handleGlobalDrop);
});

onUnmounted(() => {
  cleanupChatEvents?.();
  cleanupScreenChannel?.();
  if (!isToolWindow) {
    document.removeEventListener("keydown", handleGlobalKeydown);
  }
  document.removeEventListener("dragover", handleGlobalDragOver);
  document.removeEventListener("drop", handleGlobalDrop);
});
</script>

<template>
  <router-view />
</template>
