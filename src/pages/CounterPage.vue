<script setup lang="ts">
// 计数器页：保留原 App.vue 的 Tauri 调用（greet）逻辑，并新增一个计数器
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

// Tauri 调用相关状态（原 App.vue 功能）
const greetMsg = ref("");
const name = ref("");

async function greet() {
  // 调用 Tauri Rust 命令
  greetMsg.value = await invoke("greet", { name: name.value });
}

// 简单计数器状态
const count = ref(0);

function increment() {
  count.value++;
}

function decrement() {
  count.value--;
}

function reset() {
  count.value = 0;
}
</script>

<template>
  <main class="counter-page">
    <h1>Counter</h1>
    <p class="subtitle">保留原 App.vue 的 Tauri 调用 + 一个简单的计数器</p>

    <!-- 计数器区域 -->
    <section class="card">
      <h2>响应式计数器</h2>
      <div class="counter">
        <button class="step" aria-label="减少" @click="decrement">−</button>
        <span class="count" data-testid="count">{{ count }}</span>
        <button class="step" aria-label="增加" @click="increment">+</button>
      </div>
      <button class="reset" @click="reset">重置</button>
    </section>

    <!-- Tauri 调用区域（保留原有功能） -->
    <section class="card">
      <h2>调用 Tauri Rust 命令</h2>
      <form class="row" @submit.prevent="greet">
        <input
          id="greet-input"
          v-model="name"
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p class="greet-msg">{{ greetMsg }}</p>
      <p class="hint">
        此功能依赖 <code>src-tauri</code> 提供的 <code>greet</code> 命令，
        在纯浏览器开发模式下可能返回错误属正常现象。
      </p>
    </section>

    <RouterLink class="back" to="/">← 返回首页</RouterLink>
  </main>
</template>

<style scoped>
.counter-page {
  margin: 0;
  padding: 5vh 1.5rem 2rem;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
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
  max-width: 520px;
  background: #ffffff;
  border: 1px solid #e5e7eb;
  border-radius: 12px;
  padding: 1.5rem 1.25rem;
  margin-bottom: 1.25rem;
  text-align: left;
}

.card h2 {
  margin: 0 0 1rem;
  font-size: 1.15rem;
}

.counter {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1.25rem;
  margin-bottom: 1rem;
}

.step {
  width: 44px;
  height: 44px;
  border-radius: 50%;
  border: 1px solid #4f8cff;
  background: #ffffff;
  color: #4f8cff;
  font-size: 1.4rem;
  font-weight: 700;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.step:hover {
  background: #4f8cff;
  color: #ffffff;
}

.count {
  font-size: 2.5rem;
  font-weight: 700;
  min-width: 3ch;
  font-variant-numeric: tabular-nums;
}

.reset {
  display: block;
  margin: 0 auto;
  border-radius: 8px;
  border: 1px solid #e5e7eb;
  padding: 0.45em 1.1em;
  background: #f9fafb;
  color: #374151;
  cursor: pointer;
}

.reset:hover {
  background: #f3f4f6;
}

.row {
  display: flex;
  justify-content: center;
  gap: 0.5rem;
}

input,
button {
  border-radius: 8px;
  border: 1px solid transparent;
  padding: 0.55em 1em;
  font-size: 1em;
  font-weight: 500;
  font-family: inherit;
  color: #0f0f0f;
  background-color: #ffffff;
  transition: border-color 0.2s;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
}

button {
  cursor: pointer;
  background: #4f8cff;
  color: #ffffff;
  border-color: #4f8cff;
}

button:hover {
  background: #3a78eb;
  border-color: #3a78eb;
}

.greet-msg {
  margin: 0.75rem 0 0;
  min-height: 1.4em;
  color: #111827;
}

.hint {
  margin: 0.75rem 0 0;
  font-size: 0.85rem;
  color: #6b7280;
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

@media (prefers-color-scheme: dark) {
  .subtitle,
  .hint {
    color: #9ca3af;
  }
  .card {
    background: #1f2937;
    border-color: #374151;
  }
  .step {
    background: #1f2937;
  }
  .reset {
    background: #111827;
    color: #e5e7eb;
    border-color: #374151;
  }
  .greet-msg {
    color: #f3f4f6;
  }
  input {
    background: #111827;
    color: #f3f4f6;
  }
}
</style>