<script setup lang="ts">
/**
 * IpContainer — IcePaw 水平居中容器
 *
 * 规范:icepaw-layout-system.md v1.1 §3
 * 定位:水平居中容器,控制内容区域最大宽度(480/720/960/1200/100%)
 *
 * 默认 max-width = 'md' (720px),与 design-system §1.3.3 消息区域最大宽度一致
 *
 * 关键设计:
 *  - max-width 解析:预设 + 数字(px) + 任意 CSS 字符串(规范 §3.4.2)
 *  - padding 解析:boolean(false=0/true=md) + 复用 IpFlex 的间距语义(规范 §3.3.2)
 *  - centered:false → margin-inline: 0
 *  - fluid:true → 强制 width:100% 并忽略 maxWidth
 *  - 视口 < maxWidth 时容器自动收缩到 100%(规范 §3.5.1)
 */
import { computed } from 'vue'
import type { ContainerProps } from './types'
import type { SpaceSize } from '../flex/types'
import { resolveSpaceSize, SPACE_SIZE_MAP } from '../shared/space-size'

/* ============================================================
 * Props
 * ============================================================ */

const props = withDefaults(defineProps<ContainerProps>(), {
  maxWidth: 'md',
  centered: true,
  paddingX: 'md',
  paddingY: false,
  tag: 'div',
  fluid: false,
})

/* ============================================================
 * 常量映射
 * ============================================================ */

/** 5 档 maxWidth 预设(规范 §3.4.1) */
const MAX_WIDTH_MAP: Record<string, string> = {
  sm: '480px',
  md: '720px',  // ★ 与 design-system §1.3.3 消息区域最大宽度一致
  lg: '960px',
  xl: '1200px',
  full: '100%',
}

/* ============================================================
 * 工具函数
 * ============================================================ */

/** 解析 maxWidth(规范 §3.3.2) */
function resolveMaxWidth(value: ContainerProps['maxWidth']): string {
  if (typeof value === 'number') return `${value}px`
  if (typeof value === 'string') {
    if (value in MAX_WIDTH_MAP) return MAX_WIDTH_MAP[value]
    return value // 任意 CSS 字符串('80ch' / 'var(--my-w)' 等)
  }
  return MAX_WIDTH_MAP.md
}

/**
 * 解析 padding 值(规范 §3.3.2)
 *  - false  → '0'
 *  - true   → 'md'(= --ip-spacing-4)
 *  - 数字 / 预设 / 字符串 → 复用 shared/space-size.ts 的解析逻辑
 */
function resolvePaddingValue(value: boolean | SpaceSize | number | string | undefined): string {
  if (value === false || value === undefined) return '0'
  if (value === true) return SPACE_SIZE_MAP.md
  return resolveSpaceSize(value) ?? '0'
}

/* ============================================================
 * 派生样式
 * ============================================================ */

const rootClass = computed<string[]>(() => {
  const list: string[] = ['ip-container']
  if (!props.centered) list.push('ip-container--no-center')
  if (props.fluid) list.push('ip-container--fluid')
  return list
})

const rootStyle = computed<Record<string, string>>(() => {
  const styles: Record<string, string> = {}

  // fluid 时跳过 maxWidth(规范 §3.2 / §3.3.1)
  if (!props.fluid) {
    styles.maxWidth = resolveMaxWidth(props.maxWidth)
  }

  // padding(规范 §3.3.2)
  const px = resolvePaddingValue(props.paddingX)
  const py = resolvePaddingValue(props.paddingY)
  styles.paddingLeft = px
  styles.paddingRight = px
  styles.paddingTop = py
  styles.paddingBottom = py

  return styles
})
</script>

<template>
  <component
    :is="tag"
    :class="rootClass"
    :style="rootStyle"
  >
    <slot />
  </component>
</template>

<style scoped>
/* ============================================================
 * IpContainer — 根容器(规范 §3.3.1)
 * ============================================================ */
.ip-container {
  width: 100%;
  box-sizing: border-box;
  margin-inline: auto;       /* 默认居中 */
  /* max-width / padding 由 inline style 注入(运行计算) */
}

.ip-container--no-center {
  margin-inline: 0;          /* centered=false 时取消居中(规范 §3.7 验收) */
}

.ip-container--fluid {
  max-width: none !important;  /* fluid=true 时强制 width:100%,忽略 maxWidth(规范 §3.7) */
}
</style>
