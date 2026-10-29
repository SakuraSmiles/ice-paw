<script setup lang="ts">
// SettingsLayout.vue — 设置布局
// 顶部：统一风格头部 | 下方：左侧独立菜单卡片 + 右侧内容融入背景
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, reactive, ref, watch } from "vue";
import { useRouter, useRoute } from "vue-router";
import { Clock } from "@lucide/vue";

const router = useRouter();
const route = useRoute();

const categories = [
  { key: "general", label: "通用", en: "GENERAL", icon: "settings" },
  { key: "models", label: "模型", en: "MODELS", icon: "models" },
  { key: "agents", label: "智能体", en: "AGENTS", icon: "agent" },
  { key: "mcp", label: "工具集", en: "TOOLS", icon: "mcp" },
  { key: "kb", label: "知识库", en: "KNOWLEDGE", icon: "kb" },
  { key: "tasks", label: "定时任务", en: "TASKS", icon: "tasks" },
  { key: "logs", label: "日志", en: "LOGS", icon: "logs" },
  { key: "about", label: "关于", en: "ABOUT", icon: "about" },
];

const activeCategory = computed(() => {
  const seg = route.path.split("/").pop() || "general";
  return seg;
});

/* ===== 阴刻选中态 + 滑动切换动画 =====
   点击切换三段：阴刻收缩成侧边线（右→左，原内容淡出）→ 侧边线滑动到目标项
   （纯色）→ 侧边线展开成阴刻（左→右，新内容淡入）。淡出/淡入耗时一致（200ms）。
   首次进入（从设置外切进来）只做「展开」后半段，无滑动段——语义统一。 */

const navEl = ref<HTMLElement | null>(null);

// 视觉激活项（滞后于路由；收缩/滑动期为 null，控制阴刻态文字/图标反色）
const settledKey = ref<string | null>(null);

// 指示条几何：绝对定位在 nav 内；width 在「侧边线 2px ↔ 阴刻 100%」间过渡
const indicator = reactive({ top: 0, height: 0, width: "2px" });
const indicatorStyle = computed(() => ({
  top: `${indicator.top}px`,
  height: `${indicator.height}px`,
  width: indicator.width,
}));

// 内容淡入淡出（收缩期淡出、展开期淡入）
const contentFaded = ref(false);

// 指示条宽度动效的缓动方向：收缩（阴刻→侧边线）用 ease-in 蓄力抽回、
// 展开（侧边线→阴刻）用 spring 回弹着陆。同一元素 width 的 transition 无法按
// 方向区分缓动，故用类切换（.is-grow 时走 spring）。
const indicatorGrow = ref(true);

const prefersReduced =
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;
// 与 CSS 令牌对齐：收缩/展开 = --ip-duration-message(200ms)，滑动 = --ip-duration-modal(250ms)
const COLLAPSE_MS = prefersReduced ? 0 : 200;
const SLIDE_MS = prefersReduced ? 0 : 250;
const GROW_MS = prefersReduced ? 0 : 200;

let animating = false;
let unmounted = false;
let timer: ReturnType<typeof setTimeout> | null = null;

function sleep(ms: number) {
  return new Promise<void>((resolve) => {
    timer = setTimeout(resolve, ms);
  });
}

function nextFrame() {
  return new Promise<void>((resolve) => {
    if (typeof requestAnimationFrame !== "function") {
      resolve();
      return;
    }
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}

function measure(key: string): { top: number; height: number; width: number } {
  const nav = navEl.value;
  if (!nav) return { top: 0, height: 40, width: 160 };
  const btn = nav.querySelector<HTMLElement>(`[data-nav-key="${key}"]`);
  if (!btn) return { top: 0, height: 40, width: 160 };
  const navRect = nav.getBoundingClientRect();
  const rect = btn.getBoundingClientRect();
  return { top: rect.top - navRect.top, height: rect.height, width: rect.width };
}

// 侧边线：定位到 key 项，宽 2px
function placeLine(key: string) {
  const { top, height } = measure(key);
  indicator.top = top;
  indicator.height = height;
  indicator.width = "2px";
}

// 阴刻：定位到 key 项，宽 = 项实测宽（nav 的 padding-right 使 100% 会溢出右内边距，故用实测 px）
function placeBar(key: string) {
  const { top, height, width } = measure(key);
  indicator.top = top;
  indicator.height = height;
  indicator.width = `${width}px`;
}

async function navigate(key: string) {
  const from = settledKey.value ?? activeCategory.value;
  if (key === from || animating) return;
  animating = true;

  // 阶段一：阴刻 → 侧边线（右→左收缩，ease-in 蓄力抽回）+ 原内容淡出
  indicatorGrow.value = false;
  placeLine(from);
  settledKey.value = null;
  contentFaded.value = true;
  await sleep(COLLAPSE_MS);
  if (unmounted) return;

  // 阶段二：侧边线滑动到目标项（背景保持纯色）
  placeLine(key);
  await sleep(SLIDE_MS);
  if (unmounted) return;

  // 阶段三：切路由 + 阴刻展开（左→右，spring 回弹着陆）+ 新内容淡入
  await router.push(`/settings/${key}`);
  await nextTick();
  if (unmounted) return;
  settledKey.value = key;
  indicatorGrow.value = true;
  placeBar(key);
  contentFaded.value = false;
  await sleep(GROW_MS);
  animating = false;
}

// 外部路由变更（如侧栏设置钮跳 /settings/general）：无滑动段，只做「展开」后半段
async function arrive(key: string) {
  if (animating || key === settledKey.value) return;
  animating = true;
  indicatorGrow.value = false;
  placeLine(key);
  await nextFrame();
  settledKey.value = key;
  indicatorGrow.value = true;
  placeBar(key);
  await sleep(GROW_MS);
  animating = false;
}

onMounted(async () => {
  await nextTick();
  const key = activeCategory.value;
  indicatorGrow.value = true;
  placeLine(key);
  await nextFrame();
  // 首次进入：从侧边线展开阴刻（无滑动、无内容淡出——页面淡入由 AppLayout 承担）
  settledKey.value = key;
  placeBar(key);
});

// keep-alive 重插 DOM 后补位：深链跳转（定时任务入口 / 会话标题里的 agent 名 / 各
// CTA 按钮）走 AppLayout 的 out-in 过渡，watch→arrive 在离场期触发，此时本组件的
// nav DOM 仍脱挂，measure() 量取到零值几何 → 阴刻底不可见、反色文字不可见（白底
// 白字）。onActivated 在 DOM 重插后触发，此刻量取才是真实几何。首次挂载由 onMounted
// 的 line→bar 展开动画负责，故用 deactivatedOnce 区分「从缓存重激活」与「首次挂载」，
// 避免两路叠加出 bar→line→bar 闪烁。
let deactivatedOnce = false;
onDeactivated(() => {
  deactivatedOnce = true;
});
onActivated(() => {
  if (!deactivatedOnce) return;
  const key = activeCategory.value;
  if (!key) return;
  settledKey.value = key;
  indicatorGrow.value = true;
  placeBar(key);
});

watch(activeCategory, (newKey) => {
  if (animating) return;
  void arrive(newKey);
});

onBeforeUnmount(() => {
  unmounted = true;
  if (timer) clearTimeout(timer);
});
</script>

<template>
  <div class="settings-page">
    <!-- 顶部：与聊天头部风格统一 -->
    <header class="settings-header">
      <div class="settings-heading">
        <h1 class="header-title">设置</h1>
        <span class="header-sub">SYSTEM / SETTINGS</span>
      </div>
    </header>

    <!-- 下方：留白区域 -->
    <div class="settings-body">
      <!-- 左菜单（独立卡片） -->
      <nav ref="navEl" class="settings-nav">
        <!-- 滑动指示条：阴刻底（primary-500 纯色）与侧边线共用同一元素，
             width 在 2px ↔ 100% 间过渡，top/height 随目标项滑动 -->
        <span class="nav-indicator" :class="{ 'is-grow': indicatorGrow }" :style="indicatorStyle" aria-hidden="true"></span>
        <button
          v-for="cat in categories"
          :key="cat.key"
          :data-nav-key="cat.key"
          :class="['nav-item', { active: settledKey === cat.key }]"
          @click="navigate(cat.key)"
        >
          <span class="nav-icon">
            <!-- 设置图标 -->
            <svg v-if="cat.icon === 'settings'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
            <!-- 模型图标（layers） -->
            <svg v-else-if="cat.icon === 'models'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="12 2 2 7 12 12 22 7 12 2" /><polyline points="2 17 12 22 22 17" /><polyline points="2 12 12 17 22 12" />
            </svg>
            <!-- 智能体图标 -->
            <svg v-else-if="cat.icon === 'agent'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0 1 10 0v4" />
            </svg>
            <!-- 工具集图标 -->
            <svg v-else-if="cat.icon === 'mcp'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M9 2v6" /><path d="M15 2v6" /><path d="M6 8h12v3a6 6 0 0 1-12 0V8z" /><path d="M12 17v5" />
            </svg>
            <!-- 知识库图标 -->
            <svg v-else-if="cat.icon === 'kb'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
            </svg>
            <!-- 定时任务图标（Lucide——新代码图标一律 Lucide，存量手写渐进替换） -->
            <Clock v-else-if="cat.icon === 'tasks'" :size="16" />
            <!-- 日志图标 -->
            <svg v-else-if="cat.icon === 'logs'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="8" y1="13" x2="16" y2="13" /><line x1="8" y1="17" x2="13" y2="17" />
            </svg>
            <!-- 关于图标（info 圆） -->
            <svg v-else-if="cat.icon === 'about'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" /><path d="M12 16v-4" /><path d="M12 8h.01" />
            </svg>
          </span>
          <span class="nav-text">
            <span class="nav-label">{{ cat.label }}</span>
            <span class="nav-en">{{ cat.en }}</span>
          </span>
        </button>
      </nav>

      <!-- 右内容（融入背景） -->
      <div class="settings-content" :class="{ 'is-faded': contentFaded }">
        <router-view v-slot="{ Component }">
          <keep-alive>
            <component :is="Component" />
          </keep-alive>
        </router-view>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-page {
  height: 100%;
  display: flex;
  flex-direction: column;
  background-color: var(--ip-color-bg-primary);
}

/* ===== 统一风格头部 ===== */
.settings-header {
  display: flex;
  align-items: center;
  padding: 14px var(--ip-spacing-8);
  /* 74px = 68(原行高) + 6(高低差补偿)——让本头分割线对齐侧栏分割线（窗口顶
     110px = 36 TitleBar + 74 header），DSH 副标题放下方后保持行高不塌 */
  min-height: 74px;
  border-bottom: 1px solid var(--ip-color-border-default);
  background-color: var(--ip-color-bg-chat-header); /* blur 2026-10-28 随令牌实色化退役 */
  flex-shrink: 0;
}

/* 主标题 + 副标题上下两行（对齐 ChatHeader 的 header-info 两行结构）：
   主标题保持 18px h3；副标题 = DSH 等宽档案小字，放下方非右侧 */
.settings-heading {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.header-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.header-sub {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.24em;
  color: var(--ip-color-text-tertiary);
}

/* ===== 留白区域 ===== */
.settings-body {
  flex: 1;
  padding: var(--ip-spacing-6) var(--ip-spacing-8);
  min-height: 0;
  display: flex;
  align-items: flex-start;
  gap: var(--ip-spacing-3);
}

/* ===== 左菜单（纯文本导航） ===== */
.settings-nav {
  width: auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 78px var(--ip-spacing-8) 0 0;
  background: none;
  border: none;
  border-radius: 0;
  flex-shrink: 0;
  position: relative; /* 承载绝对定位的滑动指示条（阴刻底） */
}

/* 滑动指示条：绝对定位、z0 置于导航项之下；width 在「侧边线 2px ↔ 阴刻 100%」
   间过渡，top/height 随目标项滑动。背景恒纯色（primary-500），移动过程不加
   渐变/淡出。三相位缓动分化（治「太平淡」）——收缩 width 走 ease-in（蓄力
   抽回）、滑动 top/height 走 ease-in-out（惯性滑行）、展开 width 走 spring
   （回弹着陆）；默认态 = 收缩，.is-grow 才切 spring。 */
.nav-indicator {
  position: absolute;
  left: 0;
  z-index: 0;
  background: var(--ip-primary-500);
  border-radius: var(--ip-radius-lg);
  pointer-events: none;
  transition:
    top var(--ip-duration-modal) var(--ip-ease-in-out),
    height var(--ip-duration-modal) var(--ip-ease-in-out),
    width var(--ip-duration-message) var(--ip-ease-in);
}

.nav-indicator.is-grow {
  transition:
    top var(--ip-duration-modal) var(--ip-ease-in-out),
    height var(--ip-duration-modal) var(--ip-ease-in-out),
    width var(--ip-duration-message) var(--ip-ease-spring);
}

.nav-item {
  position: relative;
  z-index: 1; /* 文本压在指示条之上，背景透明让阴刻底透出 */
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2_5);
  width: 100%;
  padding: var(--ip-spacing-2_5) var(--ip-spacing-3) var(--ip-spacing-2_5) var(--ip-spacing-4);
  text-align: left;
  cursor: pointer;
  border: none;
  background: transparent;
  font: inherit;
  color: inherit;
}

.nav-item:hover .nav-label {
  color: var(--ip-color-text-primary);
}

/* 阴刻选中态：文字/图标反色为全局背景底色（阴刻底由 .nav-indicator 提供） */
.nav-item.active .nav-label {
  color: var(--ip-color-bg-primary);
  font-weight: var(--ip-font-weight-medium);
}

.nav-item.active .nav-en {
  color: var(--ip-color-bg-primary);
}

.nav-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
  transition: color var(--ip-duration-message) var(--ip-ease-out);
}

.nav-item.active .nav-icon {
  color: var(--ip-color-bg-primary);
}

.nav-label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  transition: color var(--ip-duration-message) var(--ip-ease-out);
}

/* 中英双行标签：中文主名 + 英文等宽副标（DSH 档案标签） */
.nav-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.nav-en {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.16em;
  color: var(--ip-color-text-disabled);
  transition: color var(--ip-duration-message) var(--ip-ease-out);
}

/* ===== 右内容 ===== */
.settings-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  align-self: stretch;
  /* 内容列统一限宽居中（GitHub/Linear settings 先例，与项目设置 tab 的 960px
     同值）：五个二级页同宽，消除「通用 780px 收窄 vs 其余全宽」切页时的宽度
     跳变割裂感；flex 剩余空间由 auto margin 吸收实现居中 */
  width: 100%;
  max-width: 960px;
  margin: 0 auto;
  /* 内容淡出（收缩期）/ 淡入（展开期）共用；耗时与收缩/展开一致（200ms） */
  transition: opacity var(--ip-duration-message) var(--ip-ease-out);
}

.settings-content.is-faded {
  opacity: 0;
}
</style>
