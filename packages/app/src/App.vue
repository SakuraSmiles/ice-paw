<script setup lang="ts">
// App.vue — 应用根组件
// 初始化全局事件监听（流式聊天事件）
import { onMounted } from "vue";
import { useChatStore } from "./stores/chat";

onMounted(() => {
  useChatStore().initEvents();
});
</script>

<template>
  <router-view />
</template>
