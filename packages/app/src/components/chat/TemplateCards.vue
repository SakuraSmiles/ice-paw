<script setup lang="ts">
// 空状态模板卡片 — 进入新会话时显示 4 个引导卡片
//
// 职责：
//   - 2×2 网格展示 4 个内置模板卡片
//   - 每个卡片：lucide icon（24px）+ 名称（14px medium）+ 描述（12px secondary）
//   - hover 高亮 + 微上浮（translateY -2px）
//   - 点击后 emit('select', template.content)
//
// props: 无
// emits:
//   - select(content: string)  用户点击某个模板卡片时

import { computed } from "vue";
import {
  Code2,
  Lightbulb,
  FileCode,
  Languages,
  type LucideIcon,
} from "lucide-vue-next";
import { BUILTIN_TEMPLATES } from "../../data/templates";

const emit = defineEmits<{
  select: [content: string];
}>();

/**
 * 图标名 → 组件 映射表。
 * BUILTIN_TEMPLATES 中 icon 字段为字符串，这里统一映射到实际组件。
 */
const ICON_MAP: Record<string, LucideIcon> = {
  Code2,
  Lightbulb,
  FileCode,
  Languages,
};

/** 带解析后图标组件的模板列表（避免在 template 中动态查找） */
const cards = computed(() =>
  BUILTIN_TEMPLATES.map((t) => ({
    ...t,
    iconComponent: ICON_MAP[t.icon] ?? Code2,
  })),
);

function onSelect(content: string): void {
  emit("select", content);
}
</script>

<template>
  <div class="template-cards">
    <p class="template-cards-title">试试这些模板开始对话 ↓</p>
    <div class="template-cards-grid">
      <button
        v-for="tpl in cards"
        :key="tpl.id"
        type="button"
        class="template-card"
        :aria-label="`使用模板：${tpl.name}`"
        @click="onSelect(tpl.content)"
      >
        <span class="template-card-icon" aria-hidden="true">
          <component :is="tpl.iconComponent" :size="24" />
        </span>
        <span class="template-card-name">{{ tpl.name }}</span>
        <span class="template-card-desc">{{ tpl.description }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.template-cards {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1 1 auto;
  padding: 32px var(--ip-spacing-5);
  background: var(--ip-color-bg-primary);
}

.template-cards-title {
  margin: 0 0 var(--ip-spacing-5);
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

.template-cards-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--ip-spacing-3, 12px);
  max-width: 480px;
  width: 100%;
}

.template-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--ip-spacing-2, 8px);
  padding: var(--ip-spacing-4, 16px);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg, 12px);
  background: var(--ip-color-bg-secondary);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition:
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.template-card:hover {
  border-color: var(--ip-color-border-strong);
  background: var(--ip-color-bg-tertiary);
  transform: translateY(-2px);
  box-shadow: 0 4px 12px -4px rgba(0, 0, 0, 0.08);
}

.template-card:active {
  transform: translateY(0);
}

.template-card:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.template-card-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--ip-radius-md, 8px);
  background: var(--ip-primary-50, #eff6ff);
  color: var(--ip-primary-600, #2563eb);
  flex-shrink: 0;
}

.template-card-name {
  font-size: var(--ip-text-body-sm-size, 14px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
}

.template-card-desc {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

/* 响应式：窄屏单列 */
@media (max-width: 400px) {
  .template-cards-grid {
    grid-template-columns: 1fr;
  }
}
</style>
