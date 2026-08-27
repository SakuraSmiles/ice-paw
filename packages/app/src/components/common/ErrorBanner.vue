<script setup lang="ts">
// ErrorBanner.vue — 统一错误反馈原语（UI-2 战役，2026-08-20 拍板双形态）
//
// 两种形态（口诀：错误贴着单个条目 → inline；关乎一组数据或整页 → banner）：
// - inline ：一行 danger 纯文本 + 可选下划线「重试」链接。条目级错误专用，
//            零装饰（无图标/底色/边框），缩进由调用方 margin 控制。
// - banner ：图标 + 底色 + 边框 + 动作组。列表级（列表顶部）与页级数据源
//            失败（页顶 + 内容降透明）专用——需要存在感与防误操作提示的场景。
//
// 文案遵循三段式（CLAUDE.md 文案规范）：title=发生了什么（必填）、
// detail=为什么/怎么办（可选）、retryLabel 动作（可选，默认「重试」）。
// dismiss 仅 banner 形态提供（inline 保持极简，错误随条目状态消亡）。
//
// Props:
//   - variant: 'inline' | 'banner'（默认 banner）
//   - title:   string 必填
//   - detail?: string 原因/提示
//   - retryLabel?: string | null —— 传 null 隐藏重试；默认「重试」
//   - actionLabel?: string —— 与重试并排的主行动按钮（如「去检查配置」）；
//               不传则不渲染。适合错误原因不在本页、需跳走处理的场景。
//   - dismissible?: boolean 仅 banner；默认 false
// Emits: retry / dismiss / action
defineProps<{
  variant?: "inline" | "banner";
  title: string;
  detail?: string;
  retryLabel?: string | null;
  actionLabel?: string;
  dismissible?: boolean;
}>();

defineEmits<{ retry: []; dismiss: []; action: [] }>();
</script>

<template>
  <!-- inline 形态：纯文本 danger 行（重试为下划线链接） -->
  <p v-if="variant === 'inline'" class="eb-inline">
    <span>{{ title }}</span><span v-if="detail">: {{ detail }}</span>
    <template v-if="retryLabel !== null">
       <button type="button" class="eb-inline-retry" @click="$emit('retry')">{{ retryLabel ?? "重试" }}</button>
    </template>
  </p>

  <!-- banner 形态：图标 + 语义底 + 动作组 -->
  <div v-else class="eb-banner" role="alert">
    <svg class="eb-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
    </svg>
    <span class="eb-text">
      <span class="eb-title">{{ title }}</span>
      <span v-if="detail" class="eb-detail">{{ detail }}</span>
    </span>
    <span class="eb-actions">
      <button v-if="actionLabel" type="button" class="eb-retry" @click="$emit('action')">{{ actionLabel }}</button>
      <button v-if="retryLabel !== null" type="button" class="eb-retry" @click="$emit('retry')">{{ retryLabel ?? "重试" }}</button>
      <button v-if="dismissible" type="button" class="eb-dismiss" title="关闭" aria-label="关闭" @click="$emit('dismiss')">×</button>
    </span>
  </div>
</template>

<style scoped>
/* ===== inline：纯文本 danger（走语义令牌，明暗自动适配） ===== */
.eb-inline {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 6px 0 0;
  padding: 0;
  font-size: 11.5px;
  line-height: 1.5;
  color: var(--ip-danger-base);
}
.eb-inline-retry {
  border: none;
  background: none;
  padding: 0;
  font: inherit;
  color: var(--ip-danger-text);
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
}
.eb-inline-retry:hover { opacity: 0.8; }

/* ===== banner：语义底 + 边框 + 动作组 ===== */
.eb-banner {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2_5);
  padding: 9px 12px;
  border-radius: 9px;
  background: var(--ip-danger-bg);
  border: 1px solid var(--ip-danger-border);
  color: var(--ip-danger-text);
  font-size: 12.5px;
  line-height: 1.5;
}
.eb-icon { flex-shrink: 0; color: var(--ip-danger-base); }
.eb-text { flex: 1; min-width: 0; }
.eb-title { font-weight: 600; }
.eb-detail { opacity: 0.82; }
.eb-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
.eb-retry {
  border: 1px solid currentColor;
  background: transparent;
  border-radius: 6px;
  padding: 3px 10px;
  font-size: 11.5px;
  font-weight: 600;
  color: inherit;
  cursor: pointer;
}
.eb-retry:hover { background: var(--ip-danger-base); color: var(--ip-color-text-on-primary); }
.eb-dismiss {
  border: none;
  background: transparent;
  padding: 2px 5px;
  font-size: 15px;
  line-height: 1;
  color: inherit;
  opacity: 0.55;
  cursor: pointer;
}
.eb-dismiss:hover { opacity: 1; }
</style>
