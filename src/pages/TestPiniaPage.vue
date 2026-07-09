<script setup lang="ts">
// Pinia 状态管理测试页：演示 counterStore 与 appStore 的 state / actions / getters
import { useAppStore } from "../stores/appStore";
import { useCounterStore } from "../stores/counterStore";

// 在 setup() 顶层调用 useXxxStore()，Pinia 会自动注入当前实例
const counterStore = useCounterStore();
const appStore = useAppStore();
</script>

<template>
  <main class="test-page">
    <h1>
      <span class="emoji">[Pinia]</span>
      Pinia 状态管理测试
    </h1>
    <p class="subtitle">验证 state / actions / getters 的响应式行为</p>

    <!-- 计数器 store 测试 -->
    <section class="card">
      <h2>counterStore · 计数器</h2>

      <div class="metric-row">
        <div class="metric">
          <span class="metric-label">当前计数</span>
          <span class="metric-value">{{ counterStore.count }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">doubleCount（getter）</span>
          <span class="metric-value accent">{{ counterStore.doubleCount }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">historyCount（getter）</span>
          <span class="metric-value">{{ counterStore.historyCount }}</span>
        </div>
      </div>

      <div class="btn-row">
        <button class="btn primary" @click="counterStore.increment">+1（increment）</button>
        <button class="btn primary" @click="counterStore.decrement">-1（decrement）</button>
        <button class="btn ghost" @click="counterStore.reset">重置（reset）</button>
      </div>

      <h3 class="sub">操作历史（history）</h3>
      <ul v-if="counterStore.history.length" class="history">
        <li v-for="(item, idx) in counterStore.history" :key="idx">
          <span class="badge">#{{ idx + 1 }}</span>
          <code>{{ item }}</code>
        </li>
      </ul>
      <p v-else class="empty">暂无历史记录，点击上方按钮试试</p>
    </section>

    <!-- 应用状态 store 测试 -->
    <section class="card">
      <h2>appStore · 应用状态</h2>

      <div class="metric-row">
        <div class="metric">
          <span class="metric-label">当前主题</span>
          <span class="metric-value" :class="appStore.theme">
            {{ appStore.theme === "light" ? "[Sun] light" : "[Moon] dark" }}
          </span>
        </div>
        <div class="metric">
          <span class="metric-label">侧边栏</span>
          <span class="metric-value">
            {{ appStore.sidebarCollapsed ? "收起" : "展开" }}
          </span>
        </div>
      </div>

      <div class="btn-row">
        <button class="btn primary" @click="appStore.toggleTheme">
          切换主题（toggleTheme）
        </button>
        <button class="btn primary" @click="appStore.toggleSidebar">
          折叠/展开侧边栏（toggleSidebar）
        </button>
      </div>

      <p class="hint">
        [Tip] 打开浏览器 DevTools，在 Counter / TestRouter 等页面切换后再回到这里，
        可以看到 <code>history</code> / <code>theme</code> / <code>sidebarCollapsed</code>
        依然被保留 —— 这就是 Pinia store 全局持久化状态的能力。
      </p>
    </section>

    <RouterLink class="back" to="/">← 返回首页</RouterLink>
  </main>
</template>

<style scoped>
.test-page {
  margin: 0;
  padding: 5vh 1.5rem 2rem;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

.emoji {
  margin-right: 0.25rem;
}

h1 {
  margin: 0;
  font-size: 2.2rem;
}

.subtitle {
  color: #6b7280;
  margin: 0.25rem 0 2rem;
}

.card {
  width: 100%;
  max-width: 680px;
  background: #ffffff;
  border: 1px solid #e5e7eb;
  border-radius: 12px;
  padding: 1.25rem 1.25rem;
  margin-bottom: 1.1rem;
  text-align: left;
}

.card h2 {
  margin: 0 0 0.75rem;
  font-size: 1.1rem;
}

.sub {
  margin: 1.25rem 0 0.5rem;
  font-size: 0.95rem;
  color: #4b5563;
}

.metric-row {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 0.6rem;
  margin-bottom: 1rem;
}

.metric {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.7rem 0.85rem;
  border-radius: 10px;
  background: #f9fafb;
  border: 1px solid #e5e7eb;
}

.metric-label {
  font-size: 0.8rem;
  color: #6b7280;
}

.metric-value {
  font-size: 1.4rem;
  font-weight: 700;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.metric-value.accent {
  color: #4f8cff;
}

.metric-value.light {
  color: #f59e0b;
}

.metric-value.dark {
  color: #6366f1;
}

.btn-row {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-top: 0.25rem;
}

.btn {
  display: inline-block;
  border-radius: 8px;
  border: 1px solid #4f8cff;
  padding: 0.5em 1em;
  font-size: 0.92em;
  font-weight: 500;
  background: #4f8cff;
  color: #ffffff;
  cursor: pointer;
  text-decoration: none;
  transition: background 0.15s ease, color 0.15s ease;
}

.btn:hover {
  background: #3a78eb;
  border-color: #3a78eb;
}

.btn.ghost {
  background: #ffffff;
  color: #4f8cff;
}

.btn.ghost:hover {
  background: #eef4ff;
}

.history {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 220px;
  overflow-y: auto;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: #f9fafb;
}

.history li {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.45rem 0.75rem;
  border-bottom: 1px solid #e5e7eb;
}

.history li:last-child {
  border-bottom: none;
}

.history .badge {
  font-size: 0.75rem;
  color: #6b7280;
  background: #ffffff;
  padding: 0.1rem 0.4rem;
  border-radius: 4px;
  border: 1px solid #e5e7eb;
}

.history code {
  background: transparent;
  font-size: 0.9rem;
}

.empty {
  margin: 0;
  padding: 0.75rem;
  color: #9ca3af;
  font-size: 0.9rem;
  background: #f9fafb;
  border-radius: 8px;
  border: 1px dashed #e5e7eb;
}

.hint {
  margin: 1rem 0 0;
  padding: 0.75rem 0.9rem;
  background: #f3f4f6;
  border-radius: 8px;
  font-size: 0.88rem;
  color: #4b5563;
}

.back {
  margin-top: 1rem;
  color: #4f8cff;
  text-decoration: none;
  font-weight: 500;
}

.back:hover {
  text-decoration: underline;
}

code {
  background: #f3f4f6;
  padding: 0.1em 0.35em;
  border-radius: 4px;
  font-size: 0.9em;
}

@media (prefers-color-scheme: dark) {
  .subtitle {
    color: #9ca3af;
  }
  .card {
    background: #1f2937;
    border-color: #374151;
  }
  .sub {
    color: #d1d5db;
  }
  .metric {
    background: #111827;
    border-color: #374151;
  }
  .metric-label {
    color: #9ca3af;
  }
  .btn.ghost {
    background: #1f2937;
  }
  .history,
  .empty {
    background: #111827;
    border-color: #374151;
  }
  .history .badge {
    background: #1f2937;
    border-color: #374151;
    color: #9ca3af;
  }
  .hint,
  code {
    background: #111827;
    color: #e5e7eb;
  }
}
</style>
