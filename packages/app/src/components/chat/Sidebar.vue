<script setup lang="ts">
// Sidebar.vue — 左侧会话列表面板
import { ref, computed, onMounted } from "vue";
import { useRouter } from "vue-router";

const router = useRouter();
const isSettingsPage = computed(() => router.currentRoute.value.path.startsWith("/settings"));

// =========================================================================
// 暗色模式
// =========================================================================

const isDark = ref(false);

function applyTheme(dark: boolean) {
  isDark.value = dark;
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
  localStorage.setItem("icepaw-theme", dark ? "dark" : "light");
}

function toggleTheme() {
  const newDark = !isDark.value;

  // View Transitions API：从按钮位置扩散切换（WebView2 Chromium 111+ 支持）
  if (document.startViewTransition) {
    const btn = document.querySelector(".btn-theme-toggle");
    const rect = btn?.getBoundingClientRect();
    const x = rect ? (rect.left + rect.width / 2) / innerWidth * 100 : 50;
    const y = rect ? (rect.top + rect.height / 2) / innerHeight * 100 : 50;

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
const showPicker = ref(false);

onMounted(() => {
  agent.load();
  chat.loadConversations();
});

function selectConv(id: string) {
  if (isSettingsPage.value) {
    router.push("/");
  }
  chat.selectConversation(id);
}

function newChat() {
  if (agent.list.length === 1) {
    doCreateChat(agent.list[0].id);
  } else {
    showPicker.value = true;
  }
}

function doCreateChat(agentId: string) {
  showPicker.value = false;
  chat.createConversation(agentId);
  if (isSettingsPage.value) {
    router.push("/");
  }
}

// 取相对时间显示
function timeAgo(dateStr: string): string {
  const d = new Date(dateStr);
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins}分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}天前`;
  return dateStr.slice(0, 10);
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
      <button class="btn-theme-toggle" @click="toggleTheme" :title="isDark ? '切换到亮色模式' : '切换到暗色模式'">
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
        <input type="text" class="search-input" placeholder="搜索对话..." />
      </div>
    </div>

    <!-- 会话列表 -->
    <nav class="conv-list">
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
      <div v-else-if="chat.conversations.length === 0 && agent.loaded" class="conv-empty">暂无对话</div>

      <button
        v-for="conv in chat.conversations"
        :key="conv.id"
        :class="['conv-item', { active: chat.activeConvId === conv.id }]"
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
          <span class="conv-time">{{ timeAgo(conv.updated_at) }}</span>
        </div>
      </button>
    </nav>

    <!-- 底部：设置等 -->
    <div class="sidebar-footer">
      <button class="footer-btn" :class="{ active: isSettingsPage }" @click="router.push('/settings/general')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span>设置</span>
      </button>
    </div>

    <!-- Agent 选择器弹窗 -->
    <AgentPicker v-if="showPicker" @select="doCreateChat" @close="showPicker = false" />
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
  color: var(--ip-primary-600);
}

.conv-item-new .conv-item-title svg {
  flex-shrink: 0;
  margin-top: 0;
}

.conv-item-new .conv-name {
  color: var(--ip-primary-600);
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

/* 底部 */
.sidebar-footer {
  padding: 8px;
  border-top: 1px solid var(--color-sidebar-border);
  flex-shrink: 0;
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
</style>
