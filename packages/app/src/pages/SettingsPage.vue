<script setup lang="ts">
// 设置页布局壳
import { computed, onMounted, onUnmounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ChevronLeft, X } from "lucide-vue-next";

const route = useRoute();
const router = useRouter();

const navItems = [
  { label: "通用", name: "SettingsGeneral", path: "/settings/general" },
  { label: "外观", name: "SettingsAppearance", path: "/settings/appearance" },
  { label: "快捷键", name: "SettingsKeyboard", path: "/settings/keyboard" },
  { label: "存储", name: "SettingsStorage", path: "/settings/storage" },
  { label: "关于", name: "SettingsAbout", path: "/settings/about" },
];

const activeNav = computed<string>(() => route.name as string);

function goBack(): void {
  router.back();
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    goBack();
  }
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div class="settings-page">
    <header class="settings-header">
      <button class="btn-icon" type="button" title="返回" aria-label="返回" @click="goBack">
        <ChevronLeft :size="18" />
      </button>
      <span class="settings-title">设置</span>
      <button class="btn-icon" type="button" title="关闭" aria-label="关闭" @click="goBack">
        <X :size="16" />
      </button>
    </header>
    <div class="settings-body">
      <nav class="settings-nav">
        <router-link
          v-for="item in navItems"
          :key="item.name"
          :to="item.path"
          :class="['nav-item', { active: activeNav === item.name }]"
        >
          {{ item.label }}
        </router-link>
      </nav>
      <div class="settings-content">
        <router-view />
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--ip-color-bg-secondary);
}
.settings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 48px;
  padding: 0 var(--ip-spacing-4);
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}
.settings-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.settings-body { display: flex; flex: 1; overflow: hidden; }
.settings-nav {
  width: 200px;
  flex-shrink: 0;
  padding: var(--ip-spacing-3) var(--ip-spacing-3);
  border-right: 1px solid var(--ip-color-border-default);
  overflow-y: auto;
}
.nav-item {
  display: block;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-size: var(--ip-text-body-size);
  color: var(--ip-color-text-secondary);
  border-radius: var(--ip-radius-md);
  text-decoration: none;
  transition: var(--ip-transition-colors);
  line-height: 1.4;
}
.nav-item:hover { color: var(--ip-color-text-primary); background-color: var(--ip-color-bg-hover); }
.nav-item.active {
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-active);
  font-weight: var(--ip-font-weight-medium);
}
.settings-content { flex: 1; overflow-y: auto; padding: var(--ip-spacing-6) var(--ip-spacing-8); }
.btn-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--ip-radius-md);
  background: transparent;
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}
.btn-icon:hover { background-color: var(--ip-color-bg-hover); color: var(--ip-color-text-primary); }
</style>
