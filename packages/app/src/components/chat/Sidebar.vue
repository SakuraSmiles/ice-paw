<script setup lang="ts">
// Sidebar.vue — 左侧会话列表面板
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useProjectStore } from "../../stores/project";
import { formatDate } from "../../utils/time";
import { useNewConversation } from "../../composables/useNewConversation";

const router = useRouter();
const project = useProjectStore();
const isSettingsPage = computed(() => router.currentRoute.value.path.startsWith("/settings"));

// 项目空间切换器
const switcherOpen = ref(false);
// 会话列表 scope 唯一真相源：当前选中的项目空间（null = 散落会话）。
// 与路由解耦：在项目里点开会话去首页聊天时，侧栏仍保持该项目 scope，不会闪回散落。
const scopeProjectId = computed(() => project.activeProjectId);
const currentProjectName = computed(() => project.activeProject?.name ?? "散落会话");
const isScopedToProject = computed(() => project.activeProjectId !== null);

function selectProject(id: string | null) {
  switcherOpen.value = false;
  project.activeProjectId = id;
  // 与「打开软件」一致：切到该空间最近一条会话；无会话则留在欢迎态。再回首页对话。
  const scoped = chat.conversations.filter((c) =>
    id === null ? !c.project_id : c.project_id === id
  );
  if (scoped.length > 0) {
    const latest = scoped.reduce((a, b) =>
      new Date(b.updated_at) > new Date(a.updated_at) ? b : a
    );
    chat.selectConversation(latest.id);
  } else {
    chat.clearActiveConversation();
  }
  router.push("/");
}

function gotoManage() {
  switcherOpen.value = false;
  router.push("/projects");
}

// =========================================================================
// 暗色模式
// =========================================================================

const isDark = ref(false);

function applyTheme(dark: boolean) {
  isDark.value = dark;
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
  localStorage.setItem("icepaw-theme", dark ? "dark" : "light");
  // 同步原生窗口主题：让 Windows 标题栏（含最小化/最大化/关闭按钮）跟随应用主题，
  // 走 Tauri 的 ImmersiveDarkMode（setTheme），与上面 DOM 的 data-theme 保持一致。
  // 非 Tauri 环境（纯 web 预览）会 reject，静默忽略。
  getCurrentWindow().setTheme(dark ? "dark" : "light").catch(() => {});
}

function toggleTheme() {
  const newDark = !isDark.value;

  // View Transitions API：从按钮位置扩散切换（WebView2 Chromium 111+ 支持）
  if (document.startViewTransition) {
    const btn = document.querySelector(".btn-theme-toggle");
    const rect = btn?.getBoundingClientRect();
    const x = rect ? (rect.left + rect.width / 2) / window.innerWidth * 100 : 50;
    const y = rect ? (rect.top + rect.height / 2) / window.innerHeight * 100 : 50;

    // 用 CSS 变量传递圆心位置，纯 CSS keyframe 驱动动画
    document.documentElement.style.setProperty("--theme-reveal-x", x + "%");
    document.documentElement.style.setProperty("--theme-reveal-y", y + "%");

    const transition = document.startViewTransition(() => {
      applyTheme(newDark);
    });

    transition.finished.finally(() => {
      document.documentElement.style.removeProperty("--theme-reveal-x");
      document.documentElement.style.removeProperty("--theme-reveal-y");
    });
  } else {
    applyTheme(newDark);
  }
}

onMounted(() => {
  const saved = localStorage.getItem("icepaw-theme");
  // 默认识别系统偏好
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  applyTheme(saved ? saved === "dark" : prefersDark);
});

// =========================================================================
// 会话列表
// =========================================================================

import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import AgentPicker from "./AgentPicker.vue";

const chat = useChatStore();
const agent = useAgentStore();
// 新建会话逻辑（与欢迎页共用 useNewConversation，保证项目内限成员一致）
const { showPicker, pickerAgentIds, startNew, onPickAgent } = useNewConversation();
const searchQuery = ref("");

const scopedConversations = computed(() => {
  const pid = scopeProjectId.value;
  return pid === null
    ? chat.conversations.filter((c) => !c.project_id)
    : chat.conversations.filter((c) => c.project_id === pid);
});

const filteredConversations = computed(() => {
  if (!searchQuery.value.trim()) return scopedConversations.value;
  const q = searchQuery.value.toLowerCase();
  return scopedConversations.value.filter((c) => {
    const agentName = agent.getById(c.agent_id)?.name?.toLowerCase() ?? "";
    return c.title?.toLowerCase().includes(q) || agentName.includes(q);
  });
});

onMounted(async () => {
  agent.load();
  project.load();
  await chat.loadConversations();
  // 打开软件时恢复上次会话（C）：有会话且未选中 → 选最近活跃的一条，
  // 并把项目空间同步到该会话所属项目，保证侧栏/切换器与当前会话一致。
  // 切项目不在此列——切项目保持「欢迎态」（用户已认可的动线）。
  if (!chat.activeConvId && chat.conversations.length > 0) {
    const latest = chat.conversations.reduce((a, b) =>
      new Date(b.updated_at) > new Date(a.updated_at) ? b : a
    );
    project.activeProjectId = latest.project_id ?? null;
    chat.selectConversation(latest.id);
  }
});

function selectConv(id: string) {
  // 非首页（设置 / 项目页）点击会话 → 回首页展示聊天
  if (router.currentRoute.value.name !== "Home") {
    router.push("/");
  }
  chat.selectConversation(id);
}

function newChat() {
  // 委托给 useNewConversation.startNew（项目内限成员、无成员引导、散落全量）
  startNew();
}

// 相对时间每分钟自动刷新：nowTick 作为 timeAgo 的响应式依赖，变化时整列重渲染
const nowTick = ref(Date.now());
let nowTickInterval: ReturnType<typeof setInterval> | null = null;
onMounted(() => {
  nowTickInterval = setInterval(() => (nowTick.value = Date.now()), 60000);
});
onUnmounted(() => {
  if (nowTickInterval) clearInterval(nowTickInterval);
});

// 取相对时间显示
function timeAgo(dateStr: string): string {
  const d = new Date(dateStr);
  // 用 nowTick.value 作「现在」基准：同时建立响应式依赖，nowTick 每分钟变化时整列重算
  const now = new Date(nowTick.value);
  const diff = now.getTime() - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins}分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}天前`;
  return formatDate(dateStr);
}
</script>

<template>
  <aside class="sidebar">
    <!-- 顶部：标题 + 暗色模式切换 -->
    <div class="sidebar-header">
      <div class="sidebar-brand" role="button" tabindex="0" @click="router.push('/')" @keydown.enter="router.push('/')">
        <span class="brand-icon">✦</span>
        <span class="brand-name">IcePaw</span>
      </div>
      <button class="btn-theme-toggle" :title="isDark ? '切换到亮色模式' : '切换到暗色模式'" @click="toggleTheme">
        <!-- 月亮（亮色模式显示 → 点击变暗）-->
        <svg v-if="!isDark" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
        </svg>
        <!-- 太阳（暗色模式显示 → 点击变亮）-->
        <svg v-else width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="5" />
          <line x1="12" y1="1" x2="12" y2="3" />
          <line x1="12" y1="21" x2="12" y2="23" />
          <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
          <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
          <line x1="1" y1="12" x2="3" y2="12" />
          <line x1="21" y1="12" x2="23" y2="12" />
          <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
          <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
        </svg>
      </button>
    </div>

    <!-- 搜索框 -->
    <div class="sidebar-search">
      <div class="search-wrapper">
        <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <input v-model="searchQuery" type="text" class="search-input" placeholder="搜索对话..." />
      </div>
    </div>

    <!-- 会话列表（TransitionGroup：会话进出淡入、touchConversation 重排时平滑让位） -->
    <TransitionGroup name="conv-list" tag="nav" class="conv-list">
      <!-- 新建对话（第一条，特殊样式） -->
      <button class="conv-item conv-item-new" @click="newChat">
        <div class="conv-item-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
          <span class="conv-name">新建对话</span>
        </div>
        <div class="conv-preview">开始一个新的对话…</div>
      </button>

      <!-- 分隔线 -->
      <div class="conv-divider"></div>

      <div v-if="chat.convLoading" class="conv-loading">加载中...</div>
      <div v-else-if="searchQuery && filteredConversations.length === 0" class="conv-empty">无匹配对话</div>
      <div v-else-if="!searchQuery && scopedConversations.length === 0 && agent.loaded" class="conv-empty">
        {{ scopeProjectId ? "项目内暂无对话" : "暂无对话" }}
      </div>

      <button
        v-for="conv in filteredConversations"
        :key="conv.id"
        :class="['conv-item', { active: chat.activeConvId === conv.id, streaming: chat.streamingConvIds.has(conv.id) }]"
        @click="selectConv(conv.id)"
      >
        <div class="conv-item-title">
          <span class="conv-name">{{ conv.title || "新对话" }}</span>
          <span v-if="conv.pinned" class="pin-icon-right" title="已置顶">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2z" /></svg>
          </span>
        </div>
        <div class="conv-meta">
          <span class="conv-agent-tag">{{ agent.getById(conv.agent_id)?.name || "未知" }}</span>
          <span v-if="chat.streamingConvIds.has(conv.id)" class="stream-indicator" title="正在生成…">
            <span class="stream-bars"><span class="bar"></span><span class="bar"></span><span class="bar"></span></span>生成中
          </span>
          <span v-else class="conv-time">{{ timeAgo(conv.updated_at) }}</span>
        </div>
      </button>
    </TransitionGroup>

    <!-- 底部：项目空间切换器（展开内含切换项 + 管理入口）/ 系统设置 -->
    <div class="sidebar-footer">
      <!-- 当前项目空间：切换器（点击展开：切换项目 + 管理全部项目） -->
      <button
        class="footer-btn proj-switcher"
        :class="{ 'switcher-open': switcherOpen, scoped: isScopedToProject }"
        :title="isScopedToProject ? `当前项目空间：${currentProjectName}` : '未选择项目（散落会话）'"
        @click="switcherOpen = !switcherOpen"
      >
        <span class="switcher-name">{{ currentProjectName }}</span>
        <svg class="switcher-caret" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="18 15 12 9 6 15" />
        </svg>
      </button>

      <!-- 设置：独立的系统设置入口 -->
      <button class="footer-btn" :class="{ active: isSettingsPage }" @click="router.push('/settings/general')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span>设置</span>
      </button>

      <!-- 项目切换弹出菜单（向上） -->
      <div v-if="switcherOpen" class="switcher-overlay" @click="switcherOpen = false" />
      <Transition name="switcher-pop">
        <nav v-if="switcherOpen" class="switcher-menu">
          <div class="switcher-header">
            <span class="switcher-title">项目空间</span>
            <button class="switcher-manage-btn" title="管理项目" @click="gotoManage">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="4" y1="21" x2="4" y2="14" /><line x1="4" y1="10" x2="4" y2="3" /><line x1="12" y1="21" x2="12" y2="12" /><line x1="12" y1="8" x2="12" y2="3" /><line x1="20" y1="21" x2="20" y2="16" /><line x1="20" y1="12" x2="20" y2="3" /><line x1="1" y1="14" x2="7" y2="14" /><line x1="9" y1="8" x2="15" y2="8" /><line x1="17" y1="16" x2="23" y2="16" />
              </svg>
            </button>
          </div>
          <div class="switcher-list">
            <button class="switcher-item" :class="{ active: !isScopedToProject }" @click="selectProject(null)">
              <span class="item-mark"><span class="item-dot muted" /></span>
              <span class="item-name">散落会话</span>
              <svg v-if="!isScopedToProject" class="item-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </button>
            <template v-if="project.list.length">
              <div class="switcher-sep" />
              <button
                v-for="p in project.list"
                :key="p.id"
                class="switcher-item"
                :class="{ active: scopeProjectId === p.id }"
                @click="selectProject(p.id)"
              >
                <span class="item-mark"><span class="item-dot" :style="p.theme_color ? { backgroundColor: p.theme_color } : {}" /></span>
                <span class="item-name">{{ p.name }}</span>
                <svg v-if="scopeProjectId === p.id" class="item-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              </button>
            </template>
          </div>
        </nav>
      </Transition>
    </div>

    <!-- Agent 选择器弹窗 -->
    <AgentPicker v-if="showPicker" :agent-ids="pickerAgentIds" @select="onPickAgent" @close="showPicker = false" />
  </aside>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-width);
  min-width: var(--sidebar-width);
  height: 100vh;
  display: flex;
  flex-direction: column;
  background-color: var(--color-sidebar-bg);
  border-right: 1px solid var(--color-sidebar-border);
  user-select: none;
}

/* 顶部区域 */
.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  min-height: 68px;
  flex-shrink: 0;
}

.sidebar-brand {
  display: flex;
  align-items: center;
  gap: 8px;
}

.brand-icon {
  font-size: 20px;
  color: var(--ip-primary-500);
}

.brand-name {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

/* 暗色模式切换按钮 */
.btn-theme-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-theme-toggle:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-primary-600);
}

/* 搜索框 */
.sidebar-search {
  padding: 0 12px 12px;
  flex-shrink: 0;
}

.search-wrapper {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 34px;
  padding: 0 10px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-md);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.search-wrapper:focus-within {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
}

.search-icon {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
}

.search-input {
  flex: 1;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  padding: 0;
  line-height: 1;
}

.search-input::placeholder {
  color: var(--ip-color-text-tertiary);
}

/* 会话列表 */
.conv-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.conv-loading, .conv-empty {
  padding: 20px 12px;
  text-align: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

.conv-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 10px 12px;
  text-align: left;
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.conv-item:hover {
  background-color: var(--color-sidebar-item-hover);
}

.conv-item.active {
  background-color: var(--color-sidebar-item-active);
}

/* 选中项左侧小圆点 */
.conv-item.active .conv-item-title::before {
  content: '';
  width: 6px;
  height: 6px;
  background-color: var(--ip-primary-500);
  border-radius: 50%;
  flex-shrink: 0;
  margin-top: 1px;
  margin-right: 2px;
}

/* 新建对话项（特殊样式） */
.conv-item-new {
  border: 1px dashed var(--ip-color-border-default);
  background-color: transparent;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.conv-item-new:hover {
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-tertiary);
}

.conv-item-new .conv-item-title {
  gap: 6px;
  color: var(--ip-color-primary-tint-text);
}

.conv-item-new .conv-item-title svg {
  flex-shrink: 0;
  margin-top: 0;
}

.conv-item-new .conv-name {
  color: var(--ip-color-primary-tint-text);
  font-weight: var(--ip-font-weight-medium);
}

.conv-item-new .conv-preview {
  padding-left: 22px;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
}

/* 分隔线 */
.conv-divider {
  height: 1px;
  background-color: var(--color-sidebar-border);
  margin: 4px 4px 2px;
}

.conv-item-title {
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  width: 100%;
}

.pin-icon-right {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  margin-left: auto;
}

.conv-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.conv-preview {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.4;
}

.conv-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
}

.conv-agent-tag {
  font-size: 11px;
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.conv-time {
  font-size: 11px;
  color: var(--ip-color-text-disabled);
  margin-left: auto;
  flex-shrink: 0;
}

/* ===== 正在生成的会话：左侧脉冲条（卡片级动画） + 「生成中」指示 ===== */
.conv-item.streaming {
  position: relative;
}
.conv-item.streaming::after {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  width: 3px;
  height: 60%;
  border-radius: 0 3px 3px 0;
  background: var(--ip-primary-500);
  transform: translateY(-50%);
  animation: conv-stream-bar 1.4s ease-in-out infinite;
}
@keyframes conv-stream-bar {
  0%, 100% { opacity: 0.35; transform: translateY(-50%) scaleY(0.6); }
  50%      { opacity: 1;    transform: translateY(-50%) scaleY(1); }
}

.stream-indicator {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
  flex-shrink: 0;
  font-size: 11px;
  color: var(--ip-color-primary-tint-text);
}
/* 三条状「生成中」指示（依次缩放，equalizer/typing 效果） */
.stream-bars {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  height: 11px;
}
.stream-bars .bar {
  width: 2px;
  height: 100%;
  border-radius: 1px;
  background: var(--ip-primary-500);
  transform-origin: center;
  animation: stream-bar-bounce 0.9s ease-in-out infinite;
}
.stream-bars .bar:nth-child(2) { animation-delay: 0.15s; }
.stream-bars .bar:nth-child(3) { animation-delay: 0.3s; }
@keyframes stream-bar-bounce {
  0%, 100% { transform: scaleY(0.35); opacity: 0.55; }
  50%      { transform: scaleY(1);    opacity: 1; }
}

/* 暗色模式下脉冲条稍亮以保证可见 */
[data-theme='dark'] .conv-item.streaming::after {
  background: var(--ip-primary-400);
}

/* 底部 */
.sidebar-footer {
  position: relative;
  padding: 8px;
  border-top: 1px solid var(--color-sidebar-border);
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.footer-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 8px 12px;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
  line-height: 1;
}

.footer-btn svg {
  display: block;
  flex-shrink: 0;
}

.footer-btn:hover,
.footer-btn.active {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-color-text-primary);
}

/* ===== 项目空间切换器：复用 footer-btn 的 ghost 语系（透明底，hover/active 才有底色） ===== */
.proj-switcher {
  width: 100%;
}
/* 展开态 = 选中底色（与会话选中项同款），表明控件处于打开 */
.proj-switcher.switcher-open {
  background-color: var(--color-sidebar-item-active);
}
/* 选中某项目时，名称提亮（轻微强调当前空间，区别于散落会话） */
.proj-switcher.scoped .switcher-name { color: var(--ip-color-text-primary); }

.switcher-name {
  flex: 1;
  min-width: 0;
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.switcher-caret {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-switcher.switcher-open .switcher-caret {
  transform: rotate(180deg);
  color: var(--ip-primary-600);
}

/* ===== 弹出菜单（向上） ===== */
.switcher-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
}

.switcher-menu {
  position: absolute;
  bottom: calc(100% + 4px);
  left: 8px;
  right: 8px;
  z-index: 51;
  padding: 0;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  display: flex;
  flex-direction: column;
  gap: 0;
}

.switcher-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border-radius: var(--ip-radius-md);
  border: none;
  background: none;
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.switcher-item:hover { background-color: var(--color-sidebar-item-hover); }
.switcher-item.active { background-color: var(--color-sidebar-item-active); }

/* 前导标记列：固定 16px 宽，让圆点 / 图标 / 文字左对齐 */
.item-mark {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.item-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background-color: var(--ip-primary-500);
}
.item-dot.muted {
  background-color: var(--ip-color-text-tertiary);
}
/* 菜单标题栏：左侧语义标题，右侧「管理」图标按钮——
   对集合的操作放标题栏，不混入下方切换列表，避免和可选项互相干扰 */
.switcher-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 10px 6px;
}
.switcher-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}
.switcher-manage-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-tertiary);
  background: none;
  border: none;
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.switcher-manage-btn:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-primary-600);
}

.switcher-list {
  padding: 0 6px 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.item-name {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.switcher-item.active .item-name { font-weight: var(--ip-font-weight-medium); }

.item-check { flex-shrink: 0; color: var(--ip-primary-600); }

.switcher-sep {
  height: 1px;
  background-color: var(--ip-color-border-default);
  margin: 4px 2px;
}

/* 弹出动画（从下方缩放进入） */
.switcher-pop-enter-active,
.switcher-pop-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
  transform-origin: bottom left;
}
.switcher-pop-enter-from,
.switcher-pop-leave-to {
  opacity: 0;
  transform: translateY(6px) scaleY(0.96);
}
</style>
