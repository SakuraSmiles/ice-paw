<script setup lang="ts">
// App.vue — 应用根组件
// 初始化全局事件监听（流式聊天事件）
import { onMounted } from "vue";
import { useChatStore } from "./stores/chat";
import { loadTimezone } from "./utils/time";

onMounted(() => {
  useChatStore().initEvents();
  // 全局加载配置时区（时间格式化的单一来源）
  loadTimezone();
});
</script>

<template>
  <router-view />
</template>
