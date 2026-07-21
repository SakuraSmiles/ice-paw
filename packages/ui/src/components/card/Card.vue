<script setup lang="ts">
/**
 * Card — IcePaw 通用卡片容器
 *
 * 规范：icepaw-p0-component-specs.md §二
 * 微交互：
 *  - interactive hover：translateY(-1px) + box-shadow-sm，150ms ease-out
 *  - interactive active：translateY(0) scale(0.99)，50ms ease-out
 *  - selected：边框 primary + 浅蓝底色，hover 不再叠加
 *  - actions slot：hover / focus-within 时 opacity 0→1，150ms
 * a11y：as='button' 时键盘可达 + Enter/Space 触发 click；selected && interactive 时 aria-pressed
 *
 * 设计要点：
 *  - 根节点 padding=0（§2.4.5 dev2 W7），由 header/body/footer 各自管理 padding
 *  - actions slot 嵌入 header 右侧，hover 显示
 */
import { computed } from 'vue'
import { resolveSpaceSize } from '../shared/space-size'
import type { CardEmits, CardProps } from './types'

const props = withDefaults(defineProps<CardProps>(), {
  variant: 'bordered',
  padding: 'md',
  interactive: false,
  selected: false,
  as: 'div',
  disabled: false,
  block: false,
})

const emit = defineEmits<CardEmits>()

/* 预设 padding 映射（§2.4.3） */
const PADDING_MAP: Record<string, string> = {
  none: '0',
  sm: 'var(--ip-card-padding-sm)',
  md: 'var(--ip-card-padding-md)',
  lg: 'var(--ip-card-padding-lg)',
}

/**
 * 解析 padding 值（§2.4.3）：
 *  - number → '${n}px'
 *  - 预设（none/sm/md/lg/xs/sm/md/lg/xl）→ PADDING_MAP / SPACE_SIZE_MAP
 *  - 其他字符串 → 原样返回（'20px' / '1rem' / 'var(--my-x)'）
 */
function resolvePadding(): string {
  if (typeof props.padding === 'number') return `${props.padding}px`
  if (typeof props.padding === 'string') {
    if (props.padding in PADDING_MAP) return PADDING_MAP[props.padding]
    const space = resolveSpaceSize(props.padding)
    return space ?? 'var(--ip-card-padding-md)'
  }
  return 'var(--ip-card-padding-md)'
}

/**
 * 根节点 padding 解析为 CSS 变量，便于 header/body/footer 复用
 * 注意：根节点自身 padding=0（W7），由各区块用 var(--ip-card-pad) 引用
 */
const paddingVar = computed<string>(() => resolvePadding())

/* W7: 根节点 padding=0，由内部 header/body/footer 各自管理 */
const rootClass = computed(() => [
  'ip-card',
  `ip-card--${props.variant}`,
  {
    'ip-card--interactive': props.interactive,
    'ip-card--selected': props.selected,
    'ip-card--disabled': props.disabled,
    'ip-card--block': props.block,
  },
])

/* 各区块 padding 通过 CSS 自定义属性传递（避免内联 style 抖动） */
const rootStyle = computed<Record<string, string>>(() => ({
  '--ip-card-pad': paddingVar.value,
}))

const isClickable = computed<boolean>(() => props.interactive && !props.disabled)

function onClick(ev: MouseEvent): void {
  if (!isClickable.value) {
    ev.preventDefault()
    return
  }
  emit('click', ev)
}
function onDblclick(ev: MouseEvent): void {
  if (!isClickable.value) return
  emit('dblclick', ev)
}
</script>

<template>
  <component
    :is="as"
    :class="rootClass"
    :style="rootStyle"
    :href="as === 'a' ? href : undefined"
    :target="as === 'a' ? target : undefined"
    :type="as === 'button' ? 'button' : undefined"
    :disabled="as === 'button' ? disabled : undefined"
    :aria-label="ariaLabel"
    :aria-pressed="interactive && selected ? 'true' : undefined"
    :aria-disabled="disabled || undefined"
    :tabindex="as === 'div' && interactive && !disabled ? 0 : undefined"
    @click="onClick"
    @dblclick="onDblclick"
  >
    <header v-if="$slots.header || $slots.actions" class="ip-card__header">
      <!-- P1-6 fix：只有 header slot 存在时才渲染 header-content div，
           否则只有 actions slot 时会出现空 header-content 占位 -->
      <div v-if="$slots.header" class="ip-card__header-content">
        <slot name="header" />
      </div>
      <div v-if="$slots.actions" class="ip-card__actions">
        <slot name="actions" />
      </div>
    </header>
    <section class="ip-card__body">
      <slot />
    </section>
    <footer v-if="$slots.footer" class="ip-card__footer">
      <slot name="footer" />
    </footer>
  </component>
</template>

<style scoped>
/* ============================================================
 * Card — 根节点（§2.4.1 / §2.4.2 / W7: padding=0）
 * ============================================================ */
.ip-card {
  display: block;
  /* 圆角（§2.4.2） */
  border-radius: var(--ip-card-radius);
  /* 字体（卡片内文本通常为 body） */
  font-family: inherit;
  color: var(--ip-color-text-body);
  /* §2.6 多属性过渡 */
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    border-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);

  box-sizing: border-box;
  text-align: left; /* 默认避免继承居中 */
  outline: none;
}

/* W7: 根节点 padding=0，由 header/body/footer 各自管理 */
/* 块级 */
.ip-card--block { width: 100%; }

/* ============================================================
 * 视觉变体（§2.4.1）
 * ============================================================ */
.ip-card--bordered {
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
}
.ip-card--filled {
  background: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
}
.ip-card--shadow {
  background: var(--ip-color-bg-secondary);
  border: 1px solid transparent;
  box-shadow: var(--ip-shadow-sm);
}

/* ============================================================
 * Header / Body / Footer（§2.4.5：各自管理 padding）
 * ============================================================ */
.ip-card__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
  /* header 顶部 padding，左右继承 */
  padding: var(--ip-card-pad) var(--ip-card-pad) 0;
}
.ip-card__header-content {
  display: flex;
  align-items: center;
  flex: 1;
  min-width: 0;
  gap: var(--ip-spacing-2);
}

.ip-card__body {
  /* body 完整 padding（W7：保证无 header/footer 时也是完整内边距） */
  padding: var(--ip-card-pad);
}

.ip-card__footer {
  /* footer 底部 padding，左右继承 */
  padding: 0 var(--ip-card-pad) var(--ip-card-pad);
}

/* ============================================================
 * Actions slot（§2.4.6：hover 时 fade-in）
 * ============================================================ */
.ip-card__actions {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  flex-shrink: 0;
  opacity: 0;
  pointer-events: none;
  transition: opacity var(--ip-duration-base) var(--ip-ease-out);
}
.ip-card:hover .ip-card__actions,
.ip-card:focus-within .ip-card__actions {
  opacity: 1;
  pointer-events: auto;
}

/* ============================================================
 * Interactive 状态（§2.4.4）
 * ============================================================ */
.ip-card--interactive {
  cursor: pointer;
}
.ip-card--interactive:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-sm);
}
.ip-card--interactive:active {
  transform: translateY(0) scale(0.99);
  /* P1-5 fix：使用语义 token 而非原始色板 */
  background: var(--ip-color-bg-tertiary);
  transition-duration: var(--ip-duration-btn-press);
}
[data-theme='dark'] .ip-card--interactive:active,
.dark .ip-card--interactive:active {
  background: var(--ip-color-bg-tertiary);
}
.ip-card--interactive:focus-visible {
  box-shadow: var(--ip-shadow-focus);
}

/* Selected 状态（§2.4.4：覆盖 hover 背景色） */
.ip-card--selected {
  border-color: var(--ip-primary-500);
  border-width: 2px;
  /* 2px 边框占位避免抖动：负向 margin 抵消 */
  margin: -1px;
  /* 选中背景用 primary 低透明度，亮暗色都适配 */
  background: color-mix(in srgb, var(--ip-primary-500) 8%, transparent);
}
.ip-card--selected.ip-card--interactive:hover {
  border-color: var(--ip-primary-500);
  /* P1-1 fix：selected 时 hover 不叠加 transform / box-shadow */
  transform: none;
  box-shadow: none;
  /* hover 时稍微加深 */
  background: color-mix(in srgb, var(--ip-primary-500) 12%, transparent);
}

/* ============================================================
 * Disabled（§2.4.4）
 * ============================================================ */
.ip-card--disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
  pointer-events: none;
}

/* ============================================================
 * button / a 原生样式重置
 * ============================================================ */
button.ip-card,
a.ip-card {
  font: inherit;
  text-decoration: none;
}
button.ip-card:focus { outline: none; }

/* ============================================================
 * 暗色模式微调
 * ============================================================ */
[data-theme='dark'] .ip-card--bordered,
.dark .ip-card--bordered {
  background: var(--ip-color-bg-secondary);
  border-color: var(--ip-color-border-default);
}
[data-theme='dark'] .ip-card--selected,
.dark .ip-card--selected {
  /* P1-5 fix：使用 primary token 的透明态代替硬编码 rgba，
     暗色 primary-400 = #60A5FA，10% alpha 提供选中反馈 */
  background: color-mix(in srgb, var(--ip-primary-400) 10%, transparent);
}
</style>