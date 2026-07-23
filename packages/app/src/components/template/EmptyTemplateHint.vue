<script setup lang="ts">
// 空状态提示组件 — Templates（spec §4.3.3）
//
// 职责：
//   - 模板列表为空时显示紧凑引导（utility persona）
//   - 提供「创建模板」入口按钮
//
// 保留行为：
//   - @create emit（兼容旧父组件）
//
// 设计要点：
//   - 紧凑单卡片（padding var(--ip-spacing-6)）
//   - 标题 sans 字体 18px + inline paw 32px
//   - 主 CTA "+ 创建模板"（保留旧 emit）
//   - 新增小提示行（Lightbulb 图标）

import { Button } from "@ice-paw/ui";
import { Plus, Lightbulb } from "lucide-vue-next";

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <div class="empty-hint" role="status" aria-live="polite">
    <!-- 标题（含 inline paw） -->
    <h2 class="empty-title">
      <span class="empty-paw-inline" aria-hidden="true">
        <svg
          class="empty-paw-svg"
          viewBox="0 0 32 32"
          xmlns="http://www.w3.org/2000/svg"
        >
          <rect width="32" height="32" rx="7" fill="var(--ip-primary-500)" />
          <g transform="translate(4, 4) scale(1.45)" fill="#FFFFFF">
            <ellipse cx="6" cy="6" rx="1.7" ry="2.3" transform="rotate(-25 6 6)" />
            <ellipse cx="11" cy="2.8" rx="1.7" ry="2.3" />
            <ellipse cx="16" cy="6" rx="1.7" ry="2.3" transform="rotate(25 16 6)" />
            <path d="M 4.5 12.5 Q 4.5 9.8, 7.8 9.3 L 14.2 9.3 Q 17.5 9.8, 17.5 12.5 Q 17.5 17.2, 11 18.2 Q 4.5 17.2, 4.5 12.5 Z" />
          </g>
        </svg>
      </span>
      <span>还没有模板</span>
    </h2>

    <p class="empty-desc">
      模板是「带变量占位符的 system prompt + user prompt 前缀」组合。<br />
      在聊天中通过 @模板名 或点击芯片注入，可大幅提升重复性任务效率。
    </p>

    <!-- 主 CTA -->
    <Button variant="primary" size="md" @click="emit('create')">
      <template #icon-left>
        <Plus :size="16" aria-hidden="true" />
      </template>
      创建模板
    </Button>

    <!-- 提示行 -->
    <p class="empty-tip">
      <Lightbulb :size="14" class="empty-tip-icon" aria-hidden="true" />
      <span>
        <strong>提示：</strong>创建 Agent 后，在会话中点 "保存为模板" 也能创建。
      </span>
    </p>
  </div>
</template>

<style scoped>
.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-6);
  text-align: center;
  gap: var(--ip-spacing-3);
  max-width: 480px;
  margin: 0 auto;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  animation: ip-empty-state-in 200ms var(--ip-ease-out) both;
}

/* ============================================================
 * 标题（含 inline paw 32px）
 * ============================================================ */
.empty-title {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin: 0;
  font-family: var(--ip-font-sans);
  font-size: 18px;
  font-weight: var(--ip-font-weight-semibold);
  line-height: 1.3;
  color: var(--ip-color-text-primary);
}

.empty-paw-inline {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  flex-shrink: 0;
}

.empty-paw-svg {
  width: 32px;
  height: 32px;
  display: block;
}

/* ============================================================
 * 描述（保留旧文案）
 * ============================================================ */
.empty-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 420px;
}

/* ============================================================
 * 提示行
 * ============================================================ */
.empty-tip {
  display: inline-flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-warning-bg);
  border: 1px solid var(--ip-warning-border);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  line-height: 1.5;
  color: var(--ip-warning-text);
  text-align: left;
  max-width: 420px;
}

.empty-tip-icon {
  flex-shrink: 0;
  margin-top: 1px;
  color: var(--ip-warning-base);
}

.empty-tip strong {
  font-weight: var(--ip-font-weight-semibold);
}
</style>