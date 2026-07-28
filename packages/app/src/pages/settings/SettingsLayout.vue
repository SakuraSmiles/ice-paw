<script setup lang="ts">
// SettingsLayout.vue — 设置布局
// 顶部：统一风格头部 | 下方：左侧独立菜单卡片 + 右侧内容融入背景
import { computed } from "vue";
import { useRouter, useRoute } from "vue-router";

const router = useRouter();
const route = useRoute();

const categories = [
  { key: "general", label: "通用", icon: "settings" },
  { key: "agents", label: "智能体", icon: "agent" },
];

const activeCategory = computed(() => {
  const seg = route.path.split("/").pop() || "general";
  return seg;
});

function navigate(key: string) {
  router.push(`/settings/${key}`);
}
</script>

<template>
  <div class="settings-page">
    <!-- 顶部：与聊天头部风格统一 -->
    <header class="settings-header">
      <h1 class="header-title">设置</h1>
    </header>

    <!-- 下方：留白区域 -->
    <div class="settings-body">
      <!-- 左菜单（独立卡片） -->
      <nav class="settings-nav">
        <button
          v-for="cat in categories"
          :key="cat.key"
          :class="['nav-item', { active: activeCategory === cat.key }]"
          @click="navigate(cat.key)"
        >
          <span class="nav-icon">
            <!-- 设置图标 -->
            <svg v-if="cat.icon === 'settings'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
            <!-- 智能体图标 -->
            <svg v-else-if="cat.icon === 'agent'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0 1 10 0v4" />
            </svg>
          </span>
          <span class="nav-label">{{ cat.label }}</span>
        </button>
      </nav>

      <!-- 右内容（融入背景） -->
      <div class="settings-content">
        <router-view />
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
  padding: 14px 32px;
  min-height: 68px;
  border-bottom: 1px solid var(--color-chat-header-border);
  background-color: var(--color-chat-header-bg);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
}

.header-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

/* ===== 留白区域 ===== */
.settings-body {
  flex: 1;
  padding: 24px 32px;
  min-height: 0;
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

/* ===== 左菜单（纯文本导航） ===== */
.settings-nav {
  width: auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 104px 32px 0 0;
  background: none;
  border: none;
  border-radius: 0;
  flex-shrink: 0;
  position: sticky;
  top: 0;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 0 8px 16px;
  text-align: left;
  cursor: pointer;
  border: none;
  border-left: 2px solid transparent;
  background: transparent;
  font: inherit;
  color: inherit;
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.nav-item:hover .nav-label {
  color: var(--ip-color-text-primary);
}

.nav-item.active {
  border-left-color: var(--ip-primary-500);
}

.nav-item.active .nav-label {
  color: var(--ip-color-text-primary);
  font-weight: var(--ip-font-weight-medium);
}

.nav-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
}

.nav-item.active .nav-icon {
  color: var(--ip-primary-500);
}

.nav-item.active .nav-icon svg {
  stroke-width: 2.5;
}

.nav-label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

/* ===== 右内容 ===== */
.settings-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}
</style>
