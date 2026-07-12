<script setup lang="ts">
/**
 * Input — IcePaw 输入框组件
 *
 * 规范：icepaw-design-system.md §2.2
 * 微交互（icepaw-micro-interactions.md §2）：
 *  - hover:    border-color 加深（gray-300 → gray-500）
 *  - focus:    border-color primary + 3px focus ring + 文字加深
 *  - error:    border danger + shake 400ms + error msg slide-in
 *  - clearable: 默认 opacity 0，hover/focus-within 时 fade-in 150ms
 *  - prefix/suffix: focus 时颜色加深
 */
import { computed, nextTick, ref, useId, watch } from 'vue'
import type { InputEmits, InputProps } from './types'

const props = withDefaults(defineProps<InputProps>(), {
  modelValue: '',
  size: 'md',
  error: false,
  disabled: false,
  readonly: false,
  clearable: false,
  type: 'text',
})

const emit = defineEmits<InputEmits>()

const internalId = useId()
const inputId = computed(() => props.inputId ?? `ip-input-${internalId}`)
const errorId = computed(() => `${inputId.value}-error`)

const focused = ref(false)
const hovered = ref(false)
const inputRef = ref<HTMLInputElement | null>(null)

/* 当前显示值 */
const displayValue = computed<string>(() => {
  if (props.modelValue === null || props.modelValue === undefined) return ''
  return String(props.modelValue)
})

/* 是否展示清空按钮（clearable + 有内容 + 非禁用/只读） */
const canClear = computed<boolean>(
  () => props.clearable && displayValue.value.length > 0 && !props.disabled && !props.readonly,
)

/* error 触发 shake：用 :key 强制重渲染以重启动画（§2.8） */
const errorKey = ref(0)
watch(
  () => props.error,
  (val) => {
    if (val) {
      errorKey.value++
      // 强制重渲染以重启动画
      nextTick(() => {
        const wrapper = inputRef.value?.closest('.ip-input')
        if (wrapper) {
          ;(wrapper as HTMLElement).style.animation = 'none'
          // eslint-disable-next-line @typescript-eslint/no-unused-expressions
          ;(wrapper as HTMLElement).offsetHeight // 触发 reflow
          ;(wrapper as HTMLElement).style.animation = ''
        }
      })
    }
  },
)

function onInput(ev: Event): void {
  const target = ev.target as HTMLInputElement
  emit('update:modelValue', target.value)
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
    emit('enter', ev)
  }
}

function onClear(ev: MouseEvent): void {
  ev.stopPropagation()
  if (props.disabled || props.readonly) return
  emit('update:modelValue', '')
  emit('clear')
  inputRef.value?.focus()
}

function onWrapperClick(): void {
  inputRef.value?.focus()
}

function onMouseEnter(): void {
  hovered.value = true
}

function onMouseLeave(): void {
  hovered.value = false
}

/* clearable 是否显示：clearable + 有内容 + (hover OR focus) */
const showClear = computed<boolean>(() => canClear.value && (hovered.value || focused.value))
</script>

<template>
  <div
    :class="[
      'ip-input',
      `ip-input--${size}`,
      {
        'ip-input--focused': focused,
        'ip-input--hovered': hovered,
        'ip-input--error': error,
        'ip-input--disabled': disabled,
        'ip-input--readonly': readonly,
      },
    ]"
    @click="onWrapperClick"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
  >
    <span v-if="$slots.prefix" class="ip-input__prefix">
      <slot name="prefix" />
    </span>

    <input
      :id="inputId"
      ref="inputRef"
      :class="['ip-input__field']"
      :type="type"
      :name="name"
      :value="displayValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :readonly="readonly"
      :maxlength="maxlength"
      :autocomplete="autocomplete"
      :aria-invalid="error || undefined"
      :aria-describedby="error && errorMessage ? errorId : undefined"
      @input="onInput"
      @focus="onFocus"
      @blur="onBlur"
      @keydown="onKeydown"
    >

    <span v-if="$slots.suffix && !showClear" class="ip-input__suffix">
      <slot name="suffix" />
    </span>

    <button
      v-if="showClear"
      type="button"
      class="ip-input__clear"
      aria-label="清空"
      tabindex="-1"
      @click="onClear"
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
        <path d="M18 6 6 18" />
        <path d="m6 6 12 12" />
      </svg>
    </button>

    <span
      v-if="error && errorMessage"
      :id="errorId"
      :key="errorKey"
      class="ip-input__error-text"
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
        class="ip-input__error-icon"
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
 * Input wrapper（§2.2）
 * ============================================================ */
.ip-input {
  display: inline-flex;
  align-items: center;
  width: 100%;
  min-height: var(--ip-input-h-md);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius);
  color: var(--ip-color-text-body);
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);

  /* §2.2 颜色过渡 100ms */
  transition:
    border-color   var(--ip-duration-fast) var(--ip-ease-out),
    box-shadow     var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);

  position: relative;
  gap: var(--ip-spacing-2);
  cursor: text;
}

/* 尺寸（§2.2） */
.ip-input--sm {
  min-height: var(--ip-input-h-sm);
  padding: var(--ip-input-py-sm) var(--ip-input-px-sm);
  font-size: var(--ip-text-body-sm-size);
}
.ip-input--lg {
  min-height: var(--ip-input-h-lg);
  padding: var(--ip-input-py-lg) var(--ip-input-px-lg);
  font-size: var(--ip-text-body-lg-size);
}

/* ---------- Hover（§2.3）border 加深 ---------- */
.ip-input--hovered:not(.ip-input--disabled):not(.ip-input--readonly):not(.ip-input--focused) {
  border-color: var(--ip-color-border-strong);
}

/* ---------- Focus（§2.4）border + shadow + 文字加深 ---------- */
.ip-input--focused {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
  color: var(--ip-color-text-primary);
}
.ip-input--focused .ip-input__field::placeholder {
  color: var(--ip-gray-300);
}

/* ---------- Error（§2.8）border + danger focus ring ---------- */
.ip-input--error {
  border-color: var(--ip-color-border-error);
  animation: ip-input-shake var(--ip-duration-shake) var(--ip-ease-in-out);
}
.ip-input--error.ip-input--focused {
  border-color: var(--ip-color-border-error);
  box-shadow: var(--ip-shadow-focus-danger);
}

/* ---------- Disabled（§2.6）---------- */
.ip-input--disabled {
  background: var(--ip-gray-100);
  border-color: var(--ip-gray-200);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
}

/* ---------- Read-only（§2.7）---------- */
.ip-input--readonly {
  background: var(--ip-gray-50);
  border-color: var(--ip-color-border-default);
  color: var(--ip-gray-600);
  cursor: default;
}

/* ============================================================
 * Native input — 透明背景，由 wrapper 承载视觉
 * ============================================================ */
.ip-input__field {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  font: inherit;
  color: inherit;
  padding: 0;
  line-height: inherit;
}

.ip-input__field::placeholder {
  color: var(--ip-color-text-disabled);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-input__field:disabled {
  cursor: not-allowed;
}

/* 隐藏原生 number input 的 spinners */
.ip-input__field[type='number']::-webkit-outer-spin-button,
.ip-input__field[type='number']::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}
.ip-input__field[type='number'] {
  -moz-appearance: textfield;
}

/* ============================================================
 * Prefix / Suffix（§2.11）focus 时颜色加深
 * ============================================================ */
.ip-input__prefix,
.ip-input__suffix {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--ip-color-icon-muted);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-input--focused .ip-input__prefix,
.ip-input--focused .ip-input__suffix {
  color: var(--ip-gray-600);
}

.ip-input__prefix svg,
.ip-input__suffix svg {
  width: 1em;
  height: 1em;
}

/* ============================================================
 * Clear button（§2.10）
 *  默认 opacity 0 + scale 0.8，仅 hover/focus-within 时显示
 *  显示时 scale → 1 微弹入
 * ============================================================ */
.ip-input__clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-right: -4px;
  flex-shrink: 0;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-full);
  color: var(--ip-color-icon-muted);
  cursor: pointer;
  opacity: 0;
  transform: scale(0.8);
  transition:
    opacity         var(--ip-duration-base) var(--ip-ease-out),
    transform       var(--ip-duration-base) var(--ip-ease-out),
    color           var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-input__clear:hover {
  color: var(--ip-gray-700);
  background: var(--ip-gray-100);
}

.ip-input__clear svg {
  width: 14px;
  height: 14px;
}

/* ============================================================
 * Error message（§2.9）slide-in 200ms
 * ============================================================ */
.ip-input__error-text {
  position: absolute;
  top: 100%;
  left: 0;
  margin-top: 6px;
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-danger-text);
  animation: ip-error-msg-in var(--ip-duration-message) var(--ip-ease-out) both;
}
.ip-input__error-icon {
  flex-shrink: 0;
  width: 12px;
  height: 12px;
}
</style>