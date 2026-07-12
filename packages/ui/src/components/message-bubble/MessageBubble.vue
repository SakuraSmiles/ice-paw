<script setup lang="ts">
/**
 * MessageBubble — IcePaw 聊天消息气泡
 *
 * 规范：icepaw-design-system.md §2.3
 * 微交互（icepaw-micro-interactions.md §4）：
 *  - enter: 首次加载 translateY(8px) + opacity，200ms ease-out
 *  - streaming: 禁用 enter 动画
 *  - 长消息: 折叠渐变遮罩 + 展开按钮
 *  - action buttons: hover/focus-within 时 fade-in
 *  - copy success: 图标 morph 为对勾 1s
 *  - 用户气泡: hover 时微弱变亮 brightness(1.05)
 *  - 失败态: danger 配色 + 重试按钮
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { MessageBubbleEmits, MessageBubbleProps } from './types'

const props = withDefaults(defineProps<MessageBubbleProps>(), {
  name: '',
  timestamp: '',
  meta: '',
  streaming: false,
  error: '',
  avatar: '',
})

const emit = defineEmits<MessageBubbleEmits>()

const displayName = computed<string>(() => {
  if (props.name) return props.name
  if (props.role === 'user') return '我'
  if (props.role === 'assistant') return 'IcePaw'
  return ''
})

const hasAvatar = computed<boolean>(() => Boolean(props.avatar))

/* ----- 长消息折叠（§4.4）----- */
const COLLAPSE_THRESHOLD = 400
const bodyRef = ref<HTMLElement | null>(null)
const isOverflow = ref(false)
const isCollapsed = ref(true)

function checkOverflow(): void {
  const el = bodyRef.value
  if (!el) return
  isOverflow.value = el.scrollHeight > COLLAPSE_THRESHOLD
  // 折叠时设 max-height 让 .is-collapsed ::after 渐变生效
  if (isCollapsed.value && isOverflow.value) {
    el.style.maxHeight = `${COLLAPSE_THRESHOLD}px`
  } else {
    el.style.maxHeight = ''
  }
}

watch(
  () => props.streaming,
  () => {
    // 流式输出不折叠
    if (props.streaming) {
      isCollapsed.value = false
      nextTick(checkOverflow)
    }
  },
)

/* 初次挂载后检查 overflow（§4.4） */
onMounted(() => {
  // 等待 slot 内容渲染
  nextTick(() => checkOverflow())
})

watch(isCollapsed, () => checkOverflow())

function toggleCollapse(): void {
  isCollapsed.value = !isCollapsed.value
}

/* ----- Copy 成功（§4.6）----- */
const isCopied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | null = null

async function onCopy(): Promise<void> {
  emit('copy')
  // 由父组件负责写入剪贴板，这里只触发 UI 反馈（除非父组件没处理）
  try {
    const text = bodyRef.value?.textContent ?? ''
    if (text && navigator?.clipboard?.writeText) {
      await navigator.clipboard.writeText(text)
    }
  } catch {
    // 忽略：父组件可能已处理
  }
  isCopied.value = true
  if (copiedTimer) clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => {
    isCopied.value = false
  }, 1000)
}

function onRegenerate(): void {
  emit('regenerate')
}

function onRetry(): void {
  emit('retry')
}
</script>

<template>
  <!-- System message：居中灰字 -->
  <div v-if="role === 'system'" class="ip-message ip-message--system">
    <div class="ip-message__system-bubble">
      <slot />
    </div>
  </div>

  <!-- User message：右侧气泡 -->
  <div
    v-else-if="role === 'user'"
    :class="[
      'ip-message',
      'ip-message--user',
      { 'ip-message--error': error },
    ]"
  >
    <div class="ip-message__content">
      <!-- header（可选：name + time） -->
      <div v-if="name !== undefined || timestamp" class="ip-message__header">
        <span v-if="name !== undefined" class="ip-message__name">{{ displayName }}</span>
        <span v-if="timestamp" class="ip-message__time">{{ timestamp }}</span>
      </div>

      <!-- bubble -->
      <div class="ip-message__bubble ip-message__bubble--user">
        <div
          ref="bodyRef"
          :class="['ip-message__body', { 'ip-message__body--collapsed': isCollapsed && isOverflow }]"
        >
          <slot />
        </div>
        <button
          v-if="isOverflow"
          type="button"
          class="ip-message__toggle"
          @click="toggleCollapse"
        >
          {{ isCollapsed ? '展开' : '收起' }}
        </button>
        <div v-if="error" class="ip-message__error-text">{{ error }}</div>
      </div>

      <!-- meta + actions -->
      <div class="ip-message__footer">
        <span v-if="timestamp" class="ip-message__meta">{{ timestamp }}</span>
        <div v-if="!$slots['footer-actions']" class="ip-message__actions">
          <button
            type="button"
            :class="['ip-message__action-btn', { 'ip-message__action-btn--copied': isCopied }]"
            aria-label="复制"
            @click="onCopy"
          >
            <svg
              v-if="!isCopied"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="ip-message__icon-copy"
              aria-hidden="true"
            >
              <rect width="14" height="14" x="8" y="8" rx="2" />
              <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
            </svg>
            <svg
              v-else
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="ip-message__icon-check"
              aria-hidden="true"
            >
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </button>
        </div>
        <slot name="footer-actions" />
      </div>
    </div>
  </div>

  <!-- Assistant message：左侧头像 + 名字 + 透明主体 -->
  <div
    v-else
    :class="[
      'ip-message',
      'ip-message--assistant',
      {
        'ip-message--streaming': streaming,
        'ip-message--error': error,
      },
    ]"
  >
    <!-- Avatar -->
    <div class="ip-message__avatar" :aria-hidden="true">
      <img v-if="hasAvatar" :src="avatar" alt="" class="ip-message__avatar-img" >
      <span v-else class="ip-message__avatar-default" aria-hidden="true">🐾</span>
    </div>

    <div class="ip-message__content">
      <!-- header -->
      <div class="ip-message__header">
        <span class="ip-message__name">{{ displayName }}</span>
        <slot name="header-actions" />
      </div>

      <!-- body：assistant 无气泡，透明 -->
      <div class="ip-message__bubble ip-message__bubble--assistant">
        <div
          ref="bodyRef"
          :class="['ip-message__body', { 'ip-message__body--collapsed': isCollapsed && isOverflow }]"
        >
          <slot />
          <span v-if="streaming && !error" class="ip-message__cursor" aria-hidden="true" />
        </div>
        <button
          v-if="isOverflow && !streaming"
          type="button"
          class="ip-message__toggle"
          @click="toggleCollapse"
        >
          {{ isCollapsed ? '展开' : '收起' }}
        </button>
        <div v-if="error" class="ip-message__error-text">
          {{ error }}
          <button
            type="button"
            class="ip-message__retry-btn"
            @click="onRetry"
          >
            重试
          </button>
        </div>
      </div>

      <!-- meta + actions -->
      <div class="ip-message__footer">
        <span v-if="timestamp" class="ip-message__meta">{{ timestamp }}</span>
        <span v-if="meta" class="ip-message__meta"> · {{ meta }}</span>
        <div v-if="!$slots['footer-actions']" class="ip-message__actions">
          <button
            type="button"
            :class="['ip-message__action-btn', { 'ip-message__action-btn--copied': isCopied }]"
            aria-label="复制"
            @click="onCopy"
          >
            <svg
              v-if="!isCopied"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="ip-message__icon-copy"
              aria-hidden="true"
            >
              <rect width="14" height="14" x="8" y="8" rx="2" />
              <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
            </svg>
            <svg
              v-else
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="ip-message__icon-check"
              aria-hidden="true"
            >
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </button>
          <button
            type="button"
            class="ip-message__action-btn"
            aria-label="重新生成"
            @click="onRegenerate"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
              <path d="M3 3v5h5" />
              <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
              <path d="M16 16h5v5" />
            </svg>
          </button>
        </div>
        <slot name="footer-actions" />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ============================================================
 * Message 容器与跨角色间距（§1.4 / §2.3）
 * ============================================================ */
.ip-message {
  display: flex;
  width: 100%;
  padding: 0 var(--ip-message-px);
  animation: ip-message-enter var(--ip-duration-message) var(--ip-ease-out) both;
}

/* 同角色 / 跨角色间距 */
.ip-message + .ip-message--user,
.ip-message--user + .ip-message--user {
  margin-top: var(--ip-message-gap-same);
}
.ip-message--assistant + .ip-message--assistant {
  margin-top: var(--ip-message-gap-same);
}
.ip-message--user + .ip-message--assistant,
.ip-message--assistant + .ip-message--user {
  margin-top: var(--ip-message-gap-cross);
}

/* 流式期间禁用滑入动画（§4.2） */
.ip-message--streaming {
  animation: none !important;
}

/* ============================================================
 * User bubble
 * ============================================================ */
.ip-message--user {
  justify-content: flex-end;
}

.ip-message--user .ip-message__content {
  max-width: var(--ip-message-user-max-w);
  display: flex;
  flex-direction: column;
  align-items: flex-end;
}

.ip-message--user .ip-message__bubble--user {
  position: relative;
  max-width: 100%;
  padding: 12px 16px;
  background: var(--ip-color-bg-user-bubble);
  color: var(--ip-color-text-on-user-bubble);
  border-radius: 12px 4px 4px 12px;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);
  word-wrap: break-word;

  /* §4.7 用户气泡悬停微亮 */
  transition: filter var(--ip-duration-base) var(--ip-ease-out);
}
.ip-message--user:hover .ip-message__bubble--user {
  filter: brightness(1.05);
}

.ip-message--user .ip-message__body :deep(*) {
  color: inherit;
}

/* ============================================================
 * Assistant bubble（无气泡感 / 透明）
 * ============================================================ */
.ip-message--assistant {
  align-items: flex-start;
}

.ip-message--assistant .ip-message__avatar {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  margin-right: 6px;
  border-radius: var(--ip-radius-full);
  background: linear-gradient(135deg, var(--ip-primary-400), var(--ip-primary-600));
  display: inline-flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  font-size: 14px;
}

.ip-message--assistant .ip-message__avatar-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.ip-message--assistant .ip-message__avatar-default {
  color: var(--ip-white);
}

.ip-message--assistant .ip-message__content {
  flex: 1;
  max-width: var(--ip-message-max-w);
  min-width: 0;
}

.ip-message--assistant .ip-message__bubble--assistant {
  position: relative;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-body);
  border-radius: 4px 12px 12px 4px;
  word-wrap: break-word;
}

/* ============================================================
 * System message
 * ============================================================ */
.ip-message--system {
  justify-content: center;
  padding-top: var(--ip-spacing-4);
  padding-bottom: var(--ip-spacing-4);
}

.ip-message__system-bubble {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

/* ============================================================
 * Header（角色名 + 时间）
 * ============================================================ */
.ip-message__header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-bottom: 4px;
}

.ip-message--user .ip-message__header {
  justify-content: flex-end;
}

.ip-message--assistant .ip-message__header {
  justify-content: flex-start;
}

.ip-message__name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.ip-message__time {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* ============================================================
 * Body — 折叠渐变（§4.4）
 * ============================================================ */
.ip-message__body {
  position: relative;
  max-height: none;
  overflow: visible;
  transition: max-height var(--ip-duration-panel) var(--ip-ease-in-out);
}
.ip-message__body--collapsed {
  max-height: 400px;
  overflow: hidden;
}
.ip-message__body--collapsed::after {
  content: '';
  position: absolute;
  inset: auto 0 0 0;
  height: 80px;
  background: linear-gradient(
    to bottom,
    transparent 0%,
    var(--ip-color-bg-secondary) 100%
  );
  pointer-events: none;
}

/* 展开按钮（§4.4）浮动 pill */
.ip-message__toggle {
  position: absolute;
  bottom: 8px;
  left: 50%;
  transform: translateX(-50%);
  padding: 4px 12px;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  z-index: 1;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color     var(--ip-duration-fast) var(--ip-ease-out),
    color            var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-message__toggle:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-primary);
}

/* ============================================================
 * Footer（meta + actions）
 * ============================================================ */
.ip-message__footer {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-top: var(--ip-message-meta-gap);
}

.ip-message--user .ip-message__footer {
  justify-content: flex-end;
  color: var(--ip-color-text-tertiary);
}

.ip-message--assistant .ip-message__footer {
  justify-content: flex-start;
  color: var(--ip-color-text-tertiary);
}

.ip-message__meta {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* ============================================================
 * Action buttons（§4.5）hover-reveal + scale active
 * ============================================================ */
.ip-message__actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  opacity: 0;
  transform: translateY(2px);
  transition:
    opacity   var(--ip-duration-base) var(--ip-ease-out),
    transform var(--ip-duration-base) var(--ip-ease-out);
}
.ip-message:hover .ip-message__actions,
.ip-message:focus-within .ip-message__actions {
  opacity: 1;
  transform: translateY(0);
}

.ip-message__action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-icon-muted);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    color            var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-message__action-btn:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-icon-default);
}
.ip-message__action-btn:active {
  transform: scale(0.92);
}
.ip-message__action-btn:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px var(--ip-focus-ring-light);
}

/* Copy 成功（§4.6） */
.ip-message__action-btn--copied {
  color: var(--ip-success-base);
}
.ip-message__icon-copy,
.ip-message__icon-check {
  display: block;
  width: 14px;
  height: 14px;
}

.ip-message__action-btn svg {
  width: 14px;
  height: 14px;
}

/* ============================================================
 * Streaming cursor（§4.3）
 * ============================================================ */
.ip-message__cursor {
  display: inline-block;
  width: 2px;
  height: 1em;
  background: var(--ip-color-text-body);
  margin-left: 2px;
  vertical-align: -2px;
  animation: ip-cursor-blink var(--ip-duration-cursor) infinite step-end;
}

/* ============================================================
 * Error state（§4.8）
 * ============================================================ */
.ip-message--error .ip-message__bubble--user {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

[data-theme='dark'] .ip-message--error .ip-message__bubble--user {
  background: var(--ip-danger-bg);     /* 暗色下 --ip-danger-bg 已是 #450A0A */
  color: var(--ip-danger-text);
}

.ip-message__error-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
  margin-top: var(--ip-spacing-1);
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.ip-message__retry-btn {
  background: transparent;
  border: 1px solid var(--ip-danger-border);
  color: var(--ip-danger-text);
  padding: 2px 8px;
  border-radius: var(--ip-radius-sm);
  font-size: var(--ip-text-caption-size);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-message__retry-btn:hover {
  background: var(--ip-danger-bg);
}
</style>