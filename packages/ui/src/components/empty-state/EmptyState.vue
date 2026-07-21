<script setup lang="ts">
/**
 * EmptyState — IcePaw 通用空状态
 *
 * 规范：icepaw-p0-component-specs.md §四
 * 微交互：
 *  - 入场：opacity + translateY 8px，400ms ease-emphasized
 *  - primaryAction / secondaryAction 复用 IpButton 规范
 *  - compact 模式：padding 减半 + 字号略小 + 图标用 md 档
 * a11y：role=status + aria-live=polite（屏幕阅读器在内容插入时朗读一次）
 */
import { computed } from 'vue'
import { FolderOpen } from 'lucide-vue-next'
import { Button as IpButton } from '../button'
import type { EmptyStateAction, EmptyStateEmits, EmptyStateIconSize, EmptyStateProps } from './types'

const props = withDefaults(defineProps<EmptyStateProps>(), {
  iconSize: 'xl',
  centered: true,
  compact: false,
})

const emit = defineEmits<EmptyStateEmits>()

/* 图标尺寸映射（§4.4.1） */
const ICON_SIZE_MAP: Record<EmptyStateIconSize, number> = {
  sm: 24,
  md: 32,
  lg: 40,
  xl: 48,
  '2xl': 64,
  '3xl': 80,
}

/** compact 时 xl 自动降为 md；其他 size 保持原值 */
const effectiveIconSize = computed<number>(() => {
  if (props.compact && props.iconSize === 'xl') return ICON_SIZE_MAP.md
  return ICON_SIZE_MAP[props.iconSize]
})

const FinalIcon = computed(() => props.icon ?? FolderOpen)

function onPrimary(ev: MouseEvent): void {
  const action: EmptyStateAction | undefined = props.primaryAction
  if (action?.onClick) action.onClick(ev)
  emit('primary', ev)
}
function onSecondary(ev: MouseEvent): void {
  const action: EmptyStateAction | undefined = props.secondaryAction
  if (action?.onClick) action.onClick(ev)
  emit('secondary', ev)
}
</script>

<template>
  <div
    :class="[
      'ip-empty-state',
      {
        'ip-empty-state--centered': centered,
        'ip-empty-state--compact': compact,
      },
    ]"
    role="status"
    :aria-label="ariaLabel ?? `${title} 空状态`"
    aria-live="polite"
  >
    <component
      :is="FinalIcon"
      class="ip-empty-state__icon"
      :size="effectiveIconSize"
      :stroke-width="1.5"
      aria-hidden="true"
    />

    <h3 v-if="title" class="ip-empty-state__title">{{ title }}</h3>
    <p v-if="description" class="ip-empty-state__description">{{ description }}</p>

    <div v-if="primaryAction || secondaryAction || $slots.actions" class="ip-empty-state__actions">
      <slot name="actions">
        <IpButton
          v-if="primaryAction"
          variant="primary"
          :size="compact ? 'sm' : 'md'"
          @click="onPrimary"
        >
          <template v-if="primaryAction.icon" #icon-left>
            <component
              :is="primaryAction.icon"
              :size="compact ? 12 : 14"
              aria-hidden="true"
            />
          </template>
          {{ primaryAction.label }}
        </IpButton>
        <IpButton
          v-if="secondaryAction"
          :variant="secondaryAction.danger ? 'danger' : 'secondary'"
          :size="compact ? 'sm' : 'md'"
          @click="onSecondary"
        >
          <template v-if="secondaryAction.icon" #icon-left>
            <component
              :is="secondaryAction.icon"
              :size="compact ? 12 : 14"
              aria-hidden="true"
            />
          </template>
          {{ secondaryAction.label }}
        </IpButton>
      </slot>
    </div>

    <slot />
  </div>
</template>

<style scoped>
/* ============================================================
 * EmptyState — 根节点（§4.4.2）
 * ============================================================ */
.ip-empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  gap: var(--ip-spacing-4);
  padding: var(--ip-spacing-8) var(--ip-spacing-4);
  max-width: 480px;
  margin: 0 auto;
  text-align: center;
  font-family: inherit;
  color: var(--ip-color-text-body);
  box-sizing: border-box;

  /* 入场动画（§4.5） */
  animation: ip-empty-state-in var(--ip-duration-page) var(--ip-ease-emphasized) both;
}

.ip-empty-state--centered {
  align-items: center;
  text-align: center;
}
.ip-empty-state--compact {
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-4);
}

/* ============================================================
 * 图标（§4.4.3）
 * ============================================================ */
.ip-empty-state__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-icon-muted);
  flex-shrink: 0;
}

/* ============================================================
 * 标题 + 描述（§4.4.3）
 * ============================================================ */
.ip-empty-state__title {
  margin: 0;
  font-size: var(--ip-text-h3-size);
  line-height: var(--ip-line-height-relaxed);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  max-width: var(--ip-empty-text-max-w);
}
.ip-empty-state--compact .ip-empty-state__title {
  font-size: var(--ip-text-body-lg-size);
}

.ip-empty-state__description {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  max-width: var(--ip-empty-text-max-w);
}
.ip-empty-state--compact .ip-empty-state__description {
  font-size: var(--ip-text-caption-size);
}

/* ============================================================
 * Actions（§4.4.4）
 * ============================================================ */
.ip-empty-state__actions {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-2);
  flex-wrap: wrap;
}
.ip-empty-state--compact .ip-empty-state__actions {
  gap: var(--ip-spacing-1_5);
}
</style>