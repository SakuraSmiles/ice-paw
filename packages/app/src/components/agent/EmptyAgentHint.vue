<script setup lang="ts">
// 空状态提示组件 — Agents（spec §4.3.2）
//
// 职责：
//   - Agent 列表为空时显示欢迎引导（专业 persona）
//   - 标题 + 描述 + 3 步骤引导 + "+ 创建 Agent" 主 CTA + 次 CTA
//
// 保留行为：
//   - @create emit（兼容旧父组件）
//
// 设计要点：
//   - paw icon 64px，padded 在 80px 圆背景内（var(--ip-primary-50)）
//   - 标题用 sans 字体（不混入 display，保持 professional persona 一致）
//   - 3 步骤引导让用户知道会发生什么（降低首次创建焦虑）

import { Button } from "@ice-paw/ui";
import { Plus, ArrowRight, UserPlus, Cpu, KeyRound } from "lucide-vue-next";
import type { Component } from "vue";

const emit = defineEmits<{
  create: [];
}>();

interface Step {
  icon: Component;
  title: string;
  desc: string;
}

const steps: Step[] = [
  {
    icon: UserPlus,
    title: "给 Agent 起名",
    desc: "显示在侧栏和消息中",
  },
  {
    icon: Cpu,
    title: "配置模型供应商",
    desc: "MiniMax / OpenAI / Anthropic 等",
  },
  {
    icon: KeyRound,
    title: "接入 API Key",
    desc: "本地加密保存，不上传服务器",
  },
];
</script>

<template>
  <div class="empty-hint" role="status" aria-live="polite">
    <!-- paw 图标 -->
    <div class="empty-paw-wrap" aria-hidden="true">
      <svg
        class="empty-paw"
        viewBox="0 0 32 32"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <linearGradient id="agentEmptyPawGrad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stop-color="var(--ip-primary-400)" />
            <stop offset="100%" stop-color="var(--ip-primary-600)" />
          </linearGradient>
        </defs>
        <rect width="32" height="32" rx="7" fill="url(#agentEmptyPawGrad)" />
        <g transform="translate(4, 4) scale(1.45)" fill="#FFFFFF">
          <ellipse cx="6" cy="6" rx="1.7" ry="2.3" transform="rotate(-25 6 6)" />
          <ellipse cx="11" cy="2.8" rx="1.7" ry="2.3" />
          <ellipse cx="16" cy="6" rx="1.7" ry="2.3" transform="rotate(25 16 6)" />
          <path d="M 4.5 12.5 Q 4.5 9.8, 7.8 9.3 L 14.2 9.3 Q 17.5 9.8, 17.5 12.5 Q 17.5 17.2, 11 18.2 Q 4.5 17.2, 4.5 12.5 Z" />
        </g>
      </svg>
    </div>

    <!-- 标题 + 描述 -->
    <h2 class="empty-title">欢迎使用 IcePaw</h2>
    <p class="empty-desc">
      Agent 是你与 LLM 之间的桥梁 — 每个 Agent 拥有独立的模型、提示词和工具配置。
    </p>

    <!-- 3 步骤引导 -->
    <ol class="empty-steps" aria-label="创建 Agent 的 3 个步骤">
      <li v-for="(s, idx) in steps" :key="idx" class="empty-step">
        <span class="empty-step-num">{{ idx + 1 }}</span>
        <span class="empty-step-icon" aria-hidden="true">
          <component :is="s.icon" :size="16" :stroke-width="2" />
        </span>
        <span class="empty-step-text">
          <span class="empty-step-title">{{ s.title }}</span>
          <span class="empty-step-desc">{{ s.desc }}</span>
        </span>
      </li>
    </ol>

    <!-- 主 CTA + 次 CTA -->
    <div class="empty-actions">
      <Button variant="primary" size="md" @click="emit('create')">
        <template #icon-left>
          <Plus :size="16" aria-hidden="true" />
        </template>
        创建 Agent
      </Button>
      <button class="empty-secondary" type="button">
        查看示例 Agent 模板
        <ArrowRight :size="14" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-12) var(--ip-spacing-6);
  text-align: center;
  gap: var(--ip-spacing-4);
  max-width: 480px;
  margin: 0 auto;
}

/* ============================================================
 * paw 图标（spec §4.3.2）
 * ============================================================ */
.empty-paw-wrap {
  width: 80px;
  height: 80px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-primary-50);
  border-radius: var(--ip-radius-full);
  padding: var(--ip-spacing-2);
  flex-shrink: 0;
}

.empty-paw {
  width: 64px;
  height: 64px;
  display: block;
}

/* ============================================================
 * 标题 + 描述（spec §4.3.2）
 * ============================================================ */
.empty-title {
  margin: 0;
  font-family: var(--ip-font-sans);
  font-size: var(--ip-text-h1-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-tight, 1.25);
  color: var(--ip-color-text-primary);
  letter-spacing: -0.01em;
}

.empty-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 320px;
}

/* ============================================================
 * 3 步骤引导（spec §4.3.2）
 * ============================================================ */
.empty-steps {
  list-style: none;
  margin: var(--ip-spacing-2) 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  width: 100%;
  max-width: 360px;
  text-align: left;
}

.empty-step {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition: border-color var(--ip-duration-base) var(--ip-ease-out);
}

.empty-step:hover {
  border-color: var(--ip-primary-300);
}

.empty-step-num {
  font-family: var(--ip-font-mono);
  font-size: 11px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  width: 18px;
  text-align: center;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}

.empty-step-icon {
  width: 28px;
  height: 28px;
  border-radius: var(--ip-radius-sm);
  background: var(--ip-primary-50);
  color: var(--ip-primary-600);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.empty-step-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.empty-step-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: 1.3;
}

.empty-step-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.3;
}

/* ============================================================
 * 主 CTA + 次 CTA
 * ============================================================ */
.empty-actions {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-top: var(--ip-spacing-2);
}

.empty-secondary {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 var(--ip-spacing-2);
  height: 28px;
  background: transparent;
  border: none;
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
  cursor: pointer;
  border-radius: var(--ip-radius-sm);
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
}

.empty-secondary:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-primary-700);
}

.empty-secondary:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}
</style>