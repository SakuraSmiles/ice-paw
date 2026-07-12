/**
 * Shared space-size utilities for layout components (IpFlex / IpContainer).
 *
 * 规范:icepaw-layout-system.md v1.1 §1.1
 *
 * 设计要点:
 *  - 单一来源的 SpaceSize 预设映射(xs/sm/md/lg/xl → CSS 变量)
 *  - 通用 resolveSpaceSize() 支持:
 *      • 数字  → '${n}px'
 *      • 预设  → SIZE_MAP[value](CSS 变量引用)
 *      • 字符串 → 原样返回(支持 '16px' / '1rem' / 'var(--my-x)')
 *  - 与 design-system §1.3 完全对齐(spacing token 已存在)
 */

import type { SpaceSize } from '../flex/types'

/** SpaceSize 预设 → CSS 变量(规范 §1.1) */
export const SPACE_SIZE_MAP: Readonly<Record<string, string>> = Object.freeze({
  xs: 'var(--ip-spacing-2)',
  sm: 'var(--ip-spacing-3)',
  md: 'var(--ip-spacing-4)',
  lg: 'var(--ip-spacing-6)',
  xl: 'var(--ip-spacing-8)',
})

/**
 * 把 SpaceSize / 数字 / 任意 CSS 字符串解析为 CSS 值
 *  - 数字:直接解释为 px
 *  - 预设字符串:查表返回 CSS 变量
 *  - 其他字符串:原样返回(支持 '16px' / '1rem' / 'var(...)')
 *  - undefined / null:返回 undefined(由调用方决定如何处理)
 */
export function resolveSpaceSize(value: SpaceSize | string | number | undefined | null): string | undefined {
  if (value === undefined || value === null) return undefined
  if (typeof value === 'number') return `${value}px`
  if (typeof value === 'string') {
    if (value in SPACE_SIZE_MAP) return SPACE_SIZE_MAP[value]
    return value
  }
  return undefined
}