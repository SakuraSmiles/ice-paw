<!-- AppLayout — 应用主布局：左侧 Sidebar + 右侧 router-view -->
<script setup lang="ts">
import { ref, onMounted } from "vue";
import Sidebar from "../chat/Sidebar.vue";

const ready = ref(false);
onMounted(() => {
  // 给浏览器一个渲染帧让 splash 先显示，再淡出
  requestAnimationFrame(() => {
    ready.value = true;
  });
});
</script>

<template>
  <!-- 启动过渡层 -->
  <Transition name="splash">
    <div v-if="!ready" class="splash-overlay">
      <div class="splash-content">
        <svg class="splash-logo" width="48" height="48" viewBox="0 0 48 48" fill="none">
          <rect width="48" height="48" rx="12" fill="var(--ip-primary-500)" />
          <path d="M14 24l6 6 14-14" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <div class="splash-dots">
          <span class="dot" />
          <span class="dot" />
          <span class="dot" />
        </div>
      </div>
    </div>
  </Transition>

  <div class="app-layout">
    <Sidebar />
    <main class="app-main">
      <router-view v-slot="{ Component }">
        <keep-alive>
          <component :is="Component" />
        </keep-alive>
      </router-view>
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

/* ===== 启动过渡层 ===== */
.splash-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-color-bg-primary);
}

.splash-content {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
}

.splash-logo {
  opacity: 0;
  animation: logo-in 0.4s ease-out 0.1s forwards;
}

@keyframes logo-in {
  from { opacity: 0; transform: scale(0.8); }
  to { opacity: 1; transform: scale(1); }
}

.splash-dots {
  display: flex;
  gap: 8px;
}

.splash-dots .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--ip-primary-400);
  animation: dot-bounce 1.2s ease-in-out infinite;
}

.splash-dots .dot:nth-child(2) { animation-delay: 0.15s; }
.splash-dots .dot:nth-child(3) { animation-delay: 0.3s; }

@keyframes dot-bounce {
  0%, 60%, 100% { opacity: 0.2; transform: scale(0.8); }
  30% { opacity: 1; transform: scale(1.2); }
}

/* 淡出过渡 */
.splash-leave-active {
  transition: opacity 0.35s ease-out;
}
.splash-leave-to {
  opacity: 0;
}
</style>
