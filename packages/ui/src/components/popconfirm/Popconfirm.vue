<script setup lang="ts">
/**
 * Popconfirm — IcePaw 轻量气泡式确认
 *
 * 规范：icepaw-p0-component-specs.md §六
 * 微交互：
 *  - 浮层 enter：opacity + scale，150ms ease-emphasized
 *  - 浮层 exit：反向 100ms ease-in
 *  - 8px CSS 箭头，朝向 placement 反方向
 *  - loading 时 confirm 自动 disabled + spinner
 * a11y：role=alertdialog + aria-labelledby/describedby + aria-haspopup=dialog
 *
 * 关键设计（W5）：
 *  - 去掉 focus trap，Tab 自然离开即关闭（避免嵌套容器抢焦点）
 *  - 打开时焦点移到 confirm 按钮；关闭时焦点回到 trigger
 *  - Esc 关闭 + emit cancel
 */
import { computed, nextTick, onMounted, onUnmounted, ref, useId, watch, type ComponentPublicInstance } from 'vue'
import { Button as IpButton } from '../button'
import type { PopconfirmEmits, PopconfirmProps } from './types'

const props = withDefaults(defineProps<PopconfirmProps>(), {
  confirmText: '确认',
  cancelText: '取消',
  danger: false,
  trigger: 'click',
  placement: 'top',
  loading: false,
  triggerSlot: 'trigger',
  width: 'auto',
})

const emit = defineEmits<PopconfirmEmits>()

const internalId = useId()
const popoverId = computed<string>(() => `ip-popconfirm-${internalId}`)
const titleId = computed<string>(() => `${popoverId.value}-title`)
const descId = computed<string>(() => `${popoverId.value}-desc`)

const open = ref<boolean>(props.modelValue)
const triggerRef = ref<HTMLElement | null>(null)
const popoverRef = ref<HTMLElement | null>(null)
/* P1-8 fix：IpButton 是组件实例，ref 类型应为 ComponentPublicInstance；
   IpButton 已通过 defineExpose 暴露 focus/blur，因此调用 .focus() 运行时正常。 */
const cancelBtnRef = ref<ComponentPublicInstance | null>(null)
const confirmBtnRef = ref<ComponentPublicInstance | null>(null)
const triggerRect = ref<DOMRect | null>(null)

watch(
  () => props.modelValue,
  (val) => {
    open.value = val
    if (val) {
      measureTrigger()
      emit('open')
      // 打开后焦点移至 confirm（ComponentPublicInstance 上 .focus 由 IpButton expose）
      nextTick(() => {
        const inst = confirmBtnRef.value as unknown as { focus?: () => void } | null
        inst?.focus?.()
      })
    } else {
      emit('close')
    }
  },
)

/* ----- 打开 / 关闭 ----- */
function openPopover(): void {
  if (open.value) return
  open.value = true
  emit('update:modelValue', true)
}
function closePopover(): void {
  if (!open.value) return
  open.value = false
  emit('update:modelValue', false)
  triggerRef.value?.focus?.()
}

/* ----- 确认 / 取消 ----- */
function onConfirm(): void {
  if (props.loading) return
  emit('confirm')
  // P0-2 fix：emit 后主动关闭浮层，不依赖父组件
  open.value = false
}
function onCancel(): void {
  if (props.loading) return
  emit('cancel')
  closePopover()
}

/* ----- 键盘（W5：Tab 自然离开，不做 trap）----- */
function onKeydown(ev: KeyboardEvent): void {
  if (!open.value) {
    if (ev.key === 'Enter' || ev.key === ' ') {
      // 注意：trigger 通常自己处理 Enter/Space；此处仅在 Popconfirm 根节点获焦时兜底
      ev.preventDefault()
      openPopover()
    }
    return
  }
  if (ev.key === 'Escape') {
    ev.preventDefault()
    if (!props.loading) onCancel()
  }
  // W5：不 trap Tab，让浏览器原生 Tab 行为把焦点带出浮层；
  // P1-3 fix：Tab 离开浮层时通过 @focusout 关闭（见模板）。
}

/* P1-3 fix：focusout 监听。
   relatedTarget 为即将获焦的元素；若为 null（焦点离开浏览器）
   或不在 popoverRef 子树内，视为 Tab 离开，自动关闭浮层。
   鼠标点击触发器不属于 focusout（mousedown 路径走 onDocumentMousedown）。 */
function onPopoverFocusOut(ev: FocusEvent): void {
  if (!open.value || props.loading) return
  const next = ev.relatedTarget as Node | null
  if (!next) {
    closePopover()
    return
  }
  if (popoverRef.value?.contains(next)) return
  // 焦点回到 trigger 也算离开浮层（用于 Esc → focus 回 trigger 的竞态防护）
  if (triggerRef.value?.contains(next)) return
  closePopover()
}

/* ----- 触发器交互 ----- */
function onTriggerClick(): void {
  if (props.trigger === 'click') {
    if (open.value) closePopover()
    else openPopover()
  }
}
function onTriggerMouseEnter(): void {
  if (props.trigger === 'hover') openPopover()
}
function onTriggerMouseLeave(): void {
  if (props.trigger === 'hover' && !props.loading) closePopover()
}

/* ----- 点击外部关闭 ----- */
function onDocumentMousedown(ev: MouseEvent): void {
  if (!open.value || props.loading) return
  const target = ev.target as Node
  if (triggerRef.value?.contains(target)) return
  if (popoverRef.value?.contains(target)) return
  closePopover()
}

/* W4：浮层打开时监听 scroll，scroll 时关闭浮层 */
function onWindowScroll(): void {
  if (open.value && !props.loading) closePopover()
}

onMounted(() => {
  document.addEventListener('mousedown', onDocumentMousedown)
  window.addEventListener('scroll', onWindowScroll, true)
  /* P1-7 fix：初始 modelValue=true 时，watch 不会触发，需要手动初始化 */
  if (props.modelValue) {
    measureTrigger()
    nextTick(() => {
      const inst = confirmBtnRef.value as unknown as { focus?: () => void } | null
      inst?.focus?.()
    })
  }
})
onUnmounted(() => {
  document.removeEventListener('mousedown', onDocumentMousedown)
  window.removeEventListener('scroll', onWindowScroll, true)
})

/* ----- 浮层定位 ----- */
function measureTrigger(): void {
  if (triggerRef.value) triggerRect.value = triggerRef.value.getBoundingClientRect()
}

const popoverStyle = computed<Record<string, string>>(() => {
  const rect = triggerRect.value
  if (!rect) return {}
  const GAP = 8
  const styles: Record<string, string> = {
    position: 'fixed',
    zIndex: 'var(--ip-z-dropdown)',
    width: typeof props.width === 'number' ? `${props.width}px` : props.width,
  }
  // 通过 bracket notation 写入 CSS 自定义属性，避开 Record<string, string>.setProperty 类型问题
  const setArrowSide = (side: 'top' | 'bottom' | 'left' | 'right'): void => {
    styles['--ip-popconfirm-arrow-side'] = side
  }
  switch (props.placement) {
    case 'top':
      styles.bottom = `${window.innerHeight - rect.top + GAP}px`
      styles.left = `${rect.left + rect.width / 2}px`
      styles.transform = 'translateX(-50%)'
      setArrowSide('bottom')
      break
    case 'bottom':
      styles.top = `${rect.bottom + GAP}px`
      styles.left = `${rect.left + rect.width / 2}px`
      styles.transform = 'translateX(-50%)'
      setArrowSide('top')
      break
    case 'left':
      styles.right = `${window.innerWidth - rect.left + GAP}px`
      styles.top = `${rect.top + rect.height / 2}px`
      styles.transform = 'translateY(-50%)'
      setArrowSide('right')
      break
    case 'right':
      styles.left = `${rect.right + GAP}px`
      styles.top = `${rect.top + rect.height / 2}px`
      styles.transform = 'translateY(-50%)'
      setArrowSide('left')
      break
  }
  return styles
})
</script>

<template>
  <div class="ip-popconfirm" @keydown="onKeydown">
    <div
      ref="triggerRef"
      class="ip-popconfirm__trigger"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="open ? popoverId : undefined"
      @click="onTriggerClick"
      @mouseenter="onTriggerMouseEnter"
      @mouseleave="onTriggerMouseLeave"
    >
      <slot name="trigger" />
    </div>

    <Teleport to="body">
      <Transition name="ip-popconfirm__popover">
        <!-- P1-4 fix：嵌套两层 div。
           外层 popoverRef-wrapper 负责定位（style transform 用 translateX/Y 居中）；
           内层 popoverRef 负责 enter/leave 动画（动画中 transform 会被覆盖）。
           否则动画结束时 transform 从 keyframe 的 translateY(0) scale(1) 跳回外层
           的 translateX(-50%)，出现位置跳变。 -->
        <div
          v-if="open"
          class="ip-popconfirm__popover-anchor"
          :style="popoverStyle"
        >
          <div
            :id="popoverId"
            ref="popoverRef"
            class="ip-popconfirm__popover"
            role="alertdialog"
            :aria-labelledby="titleId"
            :aria-describedby="description ? descId : undefined"
            aria-modal="false"
            @focusout="onPopoverFocusOut"
          >
            <div class="ip-popconfirm__arrow" aria-hidden="true" />

            <div class="ip-popconfirm__content">
              <div :id="titleId" class="ip-popconfirm__title">{{ title }}</div>
              <div
                v-if="description"
                :id="descId"
                class="ip-popconfirm__description"
              >
                {{ description }}
              </div>
            </div>

            <div class="ip-popconfirm__actions">
              <IpButton
                ref="cancelBtnRef"
                variant="secondary"
                size="sm"
                :disabled="loading"
                @click="onCancel"
              >{{ cancelText }}</IpButton>
              <IpButton
                ref="confirmBtnRef"
                :variant="danger ? 'danger' : 'primary'"
                size="sm"
                :loading="loading"
                @click="onConfirm"
              >{{ confirmText }}</IpButton>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <slot />
  </div>
</template>

<style scoped>
/* ============================================================
 * Popconfirm — 根节点
 * ============================================================ */
.ip-popconfirm {
  display: inline-flex;
  position: relative;
  font-family: inherit;
}

.ip-popconfirm__trigger {
  display: inline-flex;
  align-items: center;
  outline: none;
}

/* ============================================================
 * Popover（§6.4.1）
 * P1-4 fix：拆分为两层。
 *   popover-anchor：定位容器（fixed + transform translateX/Y 居中）。
 *   popover：动画容器（背景、阴影、圆角、动画）。动画期间 anchor 的
 *   transform 不再被覆盖，动画结束后 popover 仍处于 anchor 居中位置。
 * ============================================================ */
.ip-popconfirm__popover-anchor {
  position: fixed;
  z-index: var(--ip-z-dropdown);
  /* 宽度由内层 popover 决定，外层仅传递定位 */
}

.ip-popconfirm__popover {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  min-width: var(--ip-popconfirm-popover-min-w);
  max-width: var(--ip-popconfirm-popover-max-w);
  font-family: inherit;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.ip-popconfirm__popover-enter-active > .ip-popconfirm__popover {
  animation: ip-popover-in var(--ip-duration-base) var(--ip-ease-emphasized);
}
.ip-popconfirm__popover-leave-active > .ip-popconfirm__popover {
  animation: ip-popover-out var(--ip-duration-fast) var(--ip-ease-in);
}

/* ============================================================
 * 箭头（§6.4.4）
 * 8px CSS 三角形，用伪元素实现 + 边框模拟
 * 朝向由 --ip-popconfirm-arrow-side 决定
 * ============================================================ */
.ip-popconfirm__arrow {
  position: absolute;
  width: 8px;
  height: 8px;
  background: var(--ip-color-bg-elevated);
  /* 用 clip-path 做 8px 三角，伪元素叠加边框 */
  pointer-events: none;
  z-index: 0;
}
.ip-popconfirm__arrow::before,
.ip-popconfirm__arrow::after {
  content: '';
  position: absolute;
  inset: 0;
  width: 8px;
  height: 8px;
  background: var(--ip-color-bg-elevated);
  transform: rotate(45deg);
}
.ip-popconfirm__arrow::before {
  /* 边框层 */
  background: var(--ip-color-border-default);
}
.ip-popconfirm__arrow::after {
  /* 内部填充层（覆盖边框，仅留 1px 边） */
  inset: 1px;
  width: 6px;
  height: 6px;
  background: var(--ip-color-bg-elevated);
}

/* 箭头位置：根据 --ip-popconfirm-arrow-side 切换
   P1-4 fix：变量设置在 anchor 上（popoverStyle 传给 anchor），故选择器改为 anchor。
   箭头在 popover 内部，绝对定位不受 anchor transform 影响（除非显式继承）。 */
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side: bottom'] .ip-popconfirm__arrow,
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side:bottom'] .ip-popconfirm__arrow {
  bottom: -5px;
  left: 50%;
  margin-left: -4px;
}
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side: top'] .ip-popconfirm__arrow,
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side:top'] .ip-popconfirm__arrow {
  top: -5px;
  left: 50%;
  margin-left: -4px;
}
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side: right'] .ip-popconfirm__arrow,
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side:right'] .ip-popconfirm__arrow {
  right: -5px;
  top: 50%;
  margin-top: -4px;
}
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side: left'] .ip-popconfirm__arrow,
.ip-popconfirm__popover-anchor[style*='--ip-popconfirm-arrow-side:left'] .ip-popconfirm__arrow {
  left: -5px;
  top: 50%;
  margin-top: -4px;
}

/* ============================================================
 * 内容（§6.4.2）
 * ============================================================ */
.ip-popconfirm__content {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  position: relative;
  z-index: 1;
}

.ip-popconfirm__title {
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-relaxed);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.ip-popconfirm__description {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
}

/* ============================================================
 * Actions（§6.4.3）
 * ============================================================ */
.ip-popconfirm__actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
  position: relative;
  z-index: 1;
}
</style>