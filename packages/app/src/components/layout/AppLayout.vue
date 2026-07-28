<script setup lang="ts">
// AppLayout.vue — 应用主布局：侧栏始终保留，右侧按路由切换
import Sidebar from "../chat/Sidebar.vue";
</script>

<template>
  <div class="app-layout">
    <Sidebar />
    <main class="app-main">
      <router-view />
    </main>
  </div>
</template>

<style scoped>
.app-layout {
  display: flex;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
}

.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background-color: var(--ip-color-bg-secondary);
}
</style>
