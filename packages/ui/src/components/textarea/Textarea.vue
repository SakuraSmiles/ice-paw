<script setup lang="ts">
/**
 * Textarea — IcePaw 多行文本输入
 *
 * 规范：icepaw-design-system.md §2.2.3
 * 微交互（icepaw-micro-interactions.md §3）：
 *  - 状态（hover/focus/disabled/error）：复用 Input 的规则
 *  - auto-resize：内容变化时高度 150ms ease-out 过渡
 *  - resize handle：默认隐藏，hover wrapper 时显示（仅 resizable=true 时）
 */
import { computed, nextTick, onMounted, ref, useId, watch } from 'vue'
import type { TextareaEmits, TextareaProps } from './types'

const props = withDefaults(defineProps<TextareaProps>(), {
  modelValue: '',
  size: 'md',
  error: false,
  disabled: false,
  readonly: false,
  autoResize: false,
  resizable: false,
})

const emit = defineEmits<TextareaEmits>()

const internalId = useId()
const textareaId = computed(() => `ip-textarea-${internalId}`)
const errorId = computed(() => `${textareaId.value}-error`)

const focused = ref(false)
const hovered = ref(false)
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const overflow = ref(false)

const displayValue = computed<string>(() => {
  if (props.modelValue === null || props.modelValue === undefined) return ''
  return String(props.modelValue)
})

/* 默认 rows */
const defaultRows = computed<number>(() => {
  if (props.rows !== undefined) return props.rows
  return props.size === 'sm' ? 4 : props.size === 'lg' ? 8 : 6
})

/* 尺寸对应的最大高度 */
function getMaxHeight(size: 'sm' | 'md' | 'lg'): number {
  return size === 'sm' ? 160 : size === 'lg' ? 320 : 240
}

/* 尺寸对应的最小高度 */
function getMinHeight(size: 'sm' | 'md' | 'lg'): number {
  return size === 'sm' ? 64 : size === 'lg' ? 128 : 96
}

function onInput(ev: Event): void {
  const target = ev.target as HTMLTextAreaElement
  emit('update:modelValue', target.value)
  if (props.autoResize) {
    fitHeight()
  }
}

function onFocus(ev: FocusEvent): void {
  focused.value = true
  emit('focus', ev)
}

function onBlur(ev: FocusEvent): void {
  focused.value = false
  emit('blur', ev)
}

function onKeydown(ev: KeyboardEvent): void {
  if (ev.key === 'Enter') {
    if (!ev.shiftKey && !ev.ctrlKey && !ev.metaKey) {
      emit('enter', ev)
    }
  } else if (ev.key === 'Escape') {
    emit('escape', ev)
  }
}

function onMouseEnter(): void {
  hovered.value = true
}

function onMouseLeave(): void {
  hovered.value = false
}

/* autoResize：高度贴合内容，触顶时切 overflow-y: auto */
function fitHeight(): void {
  const el = textareaRef.value
  if (!el) return
  nextTick(() => {
    el.style.height = 'auto'
    const maxH = getMaxHeight(props.size)
    const next = Math.min(
      Math.max(el.scrollHeight, getMinHeight(props.size)),
      maxH,
    )
    el.style.height = `${next}px`
    /* 触顶时显示滚动条（§3.3） */
    overflow.value = el.scrollHeight > maxH
  })
}

watch(
  () => props.modelValue,
  () => {
    if (props.autoResize) fitHeight()
  },
)

watch(
  () => props.size,
  () => {
    if (props.autoResize) fitHeight()
  },
)

onMounted(() => {
  if (props.autoResize) fitHeight()
})
</script>

<template>
  <div
    :class="[
      'ip-textarea',
      `ip-textarea--${size}`,
      {
        'ip-textarea--focused': focused,
        'ip-textarea--hovered': hovered,
        'ip-textarea--error': error,
        'ip-textarea--disabled': disabled,
        'ip-textarea--readonly': readonly,
        'ip-textarea--auto-resize': autoResize,
        'ip-textarea--resizable': resizable,
        'ip-textarea--overflow': overflow,
      },
    ]"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
  >
    <textarea
      :id="textareaId"
      ref="textareaRef"
      :class="['ip-textarea__field']"
      :rows="defaultRows"
      :value="displayValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :readonly="readonly"
      :name="name"
      :maxlength="maxlength"
      :aria-invalid="error || undefined"
      :aria-describedby="error && errorMessage ? errorId : undefined"
      @input="onInput"
      @focus="onFocus"
      @blur="onBlur"
      @keydown="onKeydown"
    />

    <!-- 自定义 resize handle（§3.4） -->
    <div
      v-if="resizable && !autoResize"
      class="ip-textarea__resize-handle"
      aria-hidden="true"
    >
      <svg width="12" height="12" viewBox="0 0 12 12">
        <path
          d="M 11 1 L 1 11 M 11 5 L 5 11 M 11 9 L 9 11"
          stroke="currentColor"
          stroke-width="1"
          stroke-linecap="round"
          fill="none"
        />
      </svg>
    </div>

    <span
      v-if="error && errorMessage"
      :id="errorId"
      class="ip-textarea__error-text"
      role="alert"
    >
      <svg
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
        class="ip-textarea__error-icon"
      >
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      {{ errorMessage }}
    </span>
  </div>
</template>

<style scoped>
/* ============================================================
 * Textarea wrapper
 * ============================================================ */
.ip-textarea {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
}

.ip-textarea__field {
  width: 100%;
  padding: 12px 16px;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius);
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-body);
  resize: vertical;

  /* §3.2 + §3.3 border/shadow 100ms；height 150ms（auto-resize 时） */
  transition:
    border-color   var(--ip-duration-fast) var(--ip-ease-out),
    box-shadow     var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    height         var(--ip-duration-base) var(--ip-ease-out);
}

.ip-textarea__field::placeholder {
  color: var(--ip-color-text-disabled);
}

/* 尺寸（§3.2） */
.ip-textarea--sm .ip-textarea__field {
  min-height: var(--ip-textarea-min-h-sm);
  max-height: var(--ip-textarea-max-h-sm);
  padding: 8px 12px;
  font-size: var(--ip-text-body-sm-size);
}
.ip-textarea--md .ip-textarea__field {
  min-height: var(--ip-textarea-min-h-md);
  max-height: var(--ip-textarea-max-h-md);
  padding: 12px 16px;
  font-size: var(--ip-text-body-size);
}
.ip-textarea--lg .ip-textarea__field {
  min-height: var(--ip-textarea-min-h-lg);
  max-height: var(--ip-textarea-max-h-lg);
  padding: 16px 20px;
  font-size: var(--ip-text-body-size);
}

/* ---------- Auto-resize（§3.3）JS 控制高度 ---------- */
.ip-textarea--auto-resize .ip-textarea__field {
  resize: none;
  height: auto;
  overflow-y: hidden;
}
/* 触顶时显示滚动条（§3.3 JS 切换 class） */
.ip-textarea--auto-resize.ip-textarea--overflow .ip-textarea__field {
  overflow-y: auto;
}

/* ---------- Resizable handle 隐藏原生气泡，自定义手柄 ---------- */
.ip-textarea--resizable .ip-textarea__field {
  resize: none;
}

/* ============================================================
 * 状态（§3.2 复用 Input 规则）
 * ============================================================ */
.ip-textarea--hovered .ip-textarea__field:not(:disabled):not(:focus):not(:read-only) {
  border-color: var(--ip-color-border-strong);
}

.ip-textarea--focused .ip-textarea__field {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
  color: var(--ip-color-text-primary);
}
.ip-textarea--focused .ip-textarea__field::placeholder {
  color: var(--ip-gray-300);
}

.ip-textarea--error .ip-textarea__field {
  border-color: var(--ip-color-border-error);
}
.ip-textarea--error.ip-textarea--focused .ip-textarea__field {
  border-color: var(--ip-color-border-error);
  box-shadow: var(--ip-shadow-focus-danger);
}

.ip-textarea--disabled .ip-textarea__field {
  background: var(--ip-gray-100);
  border-color: var(--ip-gray-200);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
  resize: none;
}

.ip-textarea--readonly .ip-textarea__field {
  background: var(--ip-gray-50);
  border-color: var(--ip-color-border-default);
  color: var(--ip-gray-600);
  cursor: default;
  resize: none;
}

/* ============================================================
 * Resize handle（§3.4）默认 opacity 0，hover wrapper 时显示
 * 仅在 resizable=true 时渲染
 * ============================================================ */
.ip-textarea__resize-handle {
  position: absolute;
  right: 6px;
  bottom: 6px;
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-text-disabled);
  cursor: ns-resize;
  opacity: 0;
  pointer-events: none;
  transition:
    opacity   var(--ip-duration-fast) var(--ip-ease-out),
    color     var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-textarea--hovered .ip-textarea__resize-handle {
  opacity: 1;
  pointer-events: auto;
}
.ip-textarea__resize-handle:hover {
  color: var(--ip-gray-600);
}

/* ============================================================
 * Error message
 * ============================================================ */
.ip-textarea__error-text {
  margin-top: 6px;
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-danger-text);
  animation: ip-error-msg-in var(--ip-duration-message) var(--ip-ease-out) both;
}
.ip-textarea__error-icon {
  flex-shrink: 0;
  width: 12px;
  height: 12px;
}
</style>