<script setup lang="ts">
// 首页：欢迎页 + 站内导航
import { useRouter } from "vue-router";

const router = useRouter();

// 编程式导航示例
function goToCounter() {
  router.push({ name: "Counter" });
}

function goToTestRouter() {
  router.push({ name: "TestRouter" });
}

// 编程式导航示例：Pinia 测试页
function goToTestPinia() {
  router.push({ name: "TestPinia" });
}

// 编程式导航示例：SQLite 测试页
function goToTestSql() {
  router.push({ name: "TestSql" });
}

// 编程式导航示例：Keychain 加密存储测试页
function goToTestKeychain() {
  router.push({ name: "TestKeychain" });
}
</script>

<template>
  <main class="home">
    <h1 class="title">
      <span class="brand">IcePaw</span>
      <span class="paw">[IP]</span>
    </h1>
    <p class="subtitle">欢迎来到 IcePaw — 基于 Tauri + Vue 3 的桌面应用</p>

    <nav class="nav">
      <!-- 声明式导航：使用 RouterLink 组件 -->
      <RouterLink class="nav-card" to="/counter">
        <span class="nav-icon">#</span>
        <span class="nav-label">计数器</span>
        <span class="nav-hint">演示响应式状态</span>
      </RouterLink>

      <RouterLink class="nav-card" to="/test-router">
        <span class="nav-icon">&gt;</span>
        <span class="nav-label">路由测试</span>
        <span class="nav-hint">演示路由跳转 / 参数</span>
      </RouterLink>

      <RouterLink class="nav-card" to="/test-pinia">
        <span class="nav-icon">[Pinia]</span>
        <span class="nav-label">Pinia 测试</span>
        <span class="nav-hint">演示状态管理 / store</span>
      </RouterLink>

      <RouterLink class="nav-card" to="/test-sql">
        <span class="nav-icon">[SQL]</span>
        <span class="nav-label">SQLite 测试</span>
        <span class="nav-hint">演示 tauri-plugin-sql 增删改查</span>
      </RouterLink>

      <RouterLink class="nav-card" to="/test-keychain">
        <span class="nav-icon">[Keychain]</span>
        <span class="nav-label">Keychain 测试</span>
        <span class="nav-hint">演示 tauri-plugin-store 加密存储</span>
      </RouterLink>
    </nav>

    <div class="actions">
      <button class="btn" @click="goToCounter">编程式跳转 → Counter</button>
      <button class="btn" @click="goToTestRouter">编程式跳转 → TestRouter</button>
      <button class="btn" @click="goToTestPinia">编程式跳转 → TestPinia</button>
      <button class="btn" @click="goToTestSql">编程式跳转 → TestSql</button>
      <button class="btn" @click="goToTestKeychain">编程式跳转 → TestKeychain</button>
    </div>
  </main>
</template>

<style scoped>
.home {
  margin: 0;
  padding: 6vh 1.5rem 2rem;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

.title {
  font-size: 3rem;
  margin: 0 0 0.5rem;
  letter-spacing: 0.02em;
}

.brand {
  background: linear-gradient(120deg, #4f8cff, #a06bff);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  font-weight: 700;
}

.paw {
  margin-left: 0.4rem;
}

.subtitle {
  color: #6b7280;
  margin: 0 0 2.5rem;
  font-size: 1.05rem;
}

.nav {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 1rem;
  width: 100%;
  max-width: 560px;
}

.nav-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 0.25rem;
  padding: 1.1rem 1.25rem;
  border-radius: 12px;
  border: 1px solid #e5e7eb;
  background: #ffffff;
  text-decoration: none;
  color: #111827;
  transition: transform 0.15s ease, box-shadow 0.15s ease, border-color 0.15s ease;
}

.nav-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 8px 20px rgba(79, 140, 255, 0.18);
  border-color: #4f8cff;
}

.nav-icon {
  font-size: 1.6rem;
}

.nav-label {
  font-size: 1.05rem;
  font-weight: 600;
}

.nav-hint {
  font-size: 0.85rem;
  color: #6b7280;
}

.actions {
  margin-top: 2rem;
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
  justify-content: center;
}

.btn {
  border-radius: 8px;
  border: 1px solid #4f8cff;
  padding: 0.55em 1.1em;
  font-size: 0.95em;
  font-weight: 500;
  background: #4f8cff;
  color: #ffffff;
  cursor: pointer;
  transition: background 0.15s ease;
}

.btn:hover {
  background: #3a78eb;
}

@media (prefers-color-scheme: dark) {
  .subtitle,
  .nav-hint {
    color: #9ca3af;
  }
  .nav-card {
    background: #1f2937;
    border-color: #374151;
    color: #f3f4f6;
  }
}
</style>