<script setup lang="ts">
// 路由测试页：演示路由路径、编程式导航、动态参数与查询参数
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";

const route = useRoute();
const router = useRouter();

// 当前路由路径
const currentPath = computed(() => route.path);

// 动态路由参数 /test-router/:id
const routeId = computed(() => (route.params.id as string | undefined) ?? null);

// 查询参数对象（响应式）
const queryParams = computed(() => route.query);

// 预置几个 id 用于演示动态参数
const sampleIds = ["alpha", "beta", "gamma", "42"];

// 编程式导航：跳到首页
function goHome() {
  router.push({ name: "Home" });
}

// 编程式导航：跳到计数器
function goCounter() {
  router.push({ name: "Counter" });
}

// 编程式导航：跳到带动态参数的 TestRouter
function goWithId(id: string) {
  router.push({ name: "TestRouterWithId", params: { id } });
}

// 编程式导航：带查询参数
function goWithQuery() {
  router.push({
    name: "TestRouter",
    query: { foo: "bar", n: String(Date.now() % 1000) },
  });
}

// 编程式导航：替换当前历史记录（不留痕迹）
function replaceWithQuery() {
  router.replace({
    name: "TestRouter",
    query: { replaced: "yes" },
  });
}

// 后退
function goBack() {
  // 如果有历史则后退，否则去首页
  if (window.history.length > 1) {
    router.back();
  } else {
    router.push({ name: "Home" });
  }
}

// 前进
function goForward() {
  router.forward();
}
</script>

<template>
  <main class="test-page">
    <h1>路由功能测试</h1>
    <p class="subtitle">验证 <code>$route</code> / <code>router.push</code> / 动态参数 / 查询参数</p>

    <!-- 当前路由信息 -->
    <section class="card">
      <h2>当前路由信息</h2>
      <dl class="info">
        <dt>路径（path）</dt>
        <dd><code>{{ currentPath }}</code></dd>

        <dt>路由名称（name）</dt>
        <dd><code>{{ route.name ?? "—" }}</code></dd>

        <dt>动态参数（params.id）</dt>
        <dd>
          <code>{{ routeId === null ? "（无）" : routeId }}</code>
        </dd>

        <dt>查询参数（query）</dt>
        <dd>
          <pre class="query">{{ JSON.stringify(queryParams, null, 2) }}</pre>
        </dd>
      </dl>
    </section>

    <!-- 编程式导航 -->
    <section class="card">
      <h2>编程式导航</h2>
      <div class="btn-row">
        <button class="btn" @click="goHome">router.push → 首页</button>
        <button class="btn" @click="goCounter">router.push → 计数器</button>
        <button class="btn" @click="goWithQuery">router.push 带 query</button>
        <button class="btn ghost" @click="replaceWithQuery">router.replace 带 query</button>
        <button class="btn ghost" @click="goBack">router.back()</button>
        <button class="btn ghost" @click="goForward">router.forward()</button>
      </div>
    </section>

    <!-- 动态路由参数演示 -->
    <section class="card">
      <h2>动态路由参数：<code>/test-router/:id</code></h2>
      <p class="hint">点击下方按钮，观察 <code>route.params.id</code> 的变化。</p>
      <div class="btn-row">
        <button
          v-for="id in sampleIds"
          :key="id"
          class="btn"
          @click="goWithId(id)"
        >
          id = {{ id }}
        </button>
      </div>
    </section>

    <!-- 查询参数演示 -->
    <section class="card">
      <h2>查询参数：<code>?foo=bar</code></h2>
      <p class="hint">手动构造 URL 以测试任意 query。</p>
      <div class="btn-row">
        <RouterLink class="btn" :to="{ name: 'TestRouter', query: { foo: 'bar' } }">
          ?foo=bar
        </RouterLink>
        <RouterLink
          class="btn"
          :to="{ name: 'TestRouter', query: { a: '1', b: '2' } }"
        >
          ?a=1&amp;b=2
        </RouterLink>
        <RouterLink
          class="btn"
          :to="{ name: 'TestRouterWithId', params: { id: '42' }, query: { tag: 'demo' } }"
        >
          /test-router/42?tag=demo
        </RouterLink>
      </div>
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
  max-width: 640px;
  background: #ffffff;
  border: 1px solid #e5e7eb;
  border-radius: 12px;
  padding: 1.25rem 1.25rem;
  margin-bottom: 1.1rem;
  text-align: left;
}

.card h2 {
  margin: 0 0 0.75rem;
  font-size: 1.05rem;
}

.info {
  display: grid;
  grid-template-columns: 140px 1fr;
  gap: 0.5rem 1rem;
  margin: 0;
}

.info dt {
  color: #6b7280;
  font-weight: 500;
}

.info dd {
  margin: 0;
  word-break: break-all;
}

.query {
  margin: 0;
  padding: 0.6rem 0.75rem;
  background: #f3f4f6;
  border-radius: 6px;
  font-size: 0.85rem;
  overflow-x: auto;
}

.btn-row {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
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

.hint {
  margin: 0 0 0.75rem;
  color: #6b7280;
  font-size: 0.88rem;
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
  .subtitle,
  .hint,
  .info dt {
    color: #9ca3af;
  }
  .card {
    background: #1f2937;
    border-color: #374151;
  }
  .btn.ghost {
    background: #1f2937;
  }
  .query,
  code {
    background: #111827;
    color: #e5e7eb;
  }
}
</style>