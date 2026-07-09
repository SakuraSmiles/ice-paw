<script setup lang="ts">
// App.vue 现在只作为路由出口，全局导航放在此处
import { RouterLink, RouterView } from "vue-router";
</script>

<template>
  <div class="app-shell">
    <!-- 顶部导航栏 -->
    <header class="app-header">
      <RouterLink class="brand" to="/">
        <span class="brand-text">IcePaw</span>
        <span class="brand-paw">[IP]</span>
      </RouterLink>
      <nav class="primary-nav">
        <RouterLink class="nav-link" to="/">Home</RouterLink>
        <RouterLink class="nav-link" to="/counter">Counter</RouterLink>
        <RouterLink class="nav-link" to="/test-router">Test Router</RouterLink>
      </nav>
    </header>

    <!-- 路由出口：页面组件在这里渲染 -->
    <RouterView v-slot="{ Component }">
      <component :is="Component" />
    </RouterView>
  </div>
</template>

<style scoped>
.app-shell {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
}

.app-header {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.75rem 1.5rem;
  background: rgba(255, 255, 255, 0.85);
  backdrop-filter: saturate(180%) blur(12px);
  border-bottom: 1px solid #e5e7eb;
}

.brand {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  font-size: 1.1rem;
  font-weight: 700;
  text-decoration: none;
  color: inherit;
}

.brand-text {
  background: linear-gradient(120deg, #4f8cff, #a06bff);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.primary-nav {
  display: flex;
  gap: 0.25rem;
}

.nav-link {
  padding: 0.45rem 0.85rem;
  border-radius: 8px;
  font-size: 0.95rem;
  font-weight: 500;
  text-decoration: none;
  color: #4b5563;
  transition: background 0.15s ease, color 0.15s ease;
}

.nav-link:hover {
  background: #f3f4f6;
  color: #111827;
}

/* RouterLink 默认会给当前激活的链接加上 .router-link-active 类 */
.nav-link.router-link-active {
  background: #4f8cff;
  color: #ffffff;
}

.nav-link.router-link-exact-active {
  background: #3a78eb;
  color: #ffffff;
}

@media (prefers-color-scheme: dark) {
  .app-header {
    background: rgba(17, 24, 39, 0.85);
    border-bottom-color: #1f2937;
  }
  .nav-link {
    color: #d1d5db;
  }
  .nav-link:hover {
    background: #111827;
    color: #f3f4f6;
  }
}
</style>