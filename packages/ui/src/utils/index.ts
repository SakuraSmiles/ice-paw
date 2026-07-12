/**
 * IcePaw utils — 提供 props / events / a11y 工具
 */

import { computed, type CSSProperties, type ComputedRef } from 'vue'

/* ============================================================
 * buildProps — props 标准化 + 类型推导
 * ============================================================ */

type PropType<T> = T | null | undefined

type ResolveProp<T> = {
  get value(): PropType<T>
}

type ExtractPropTypes<O> = {
  [K in keyof O]: O[K] extends ResolveProp<infer V> ? V : never
}

type PublicPropDefinitions<O> = {
  readonly [K in keyof O]: O[K] extends { readonly type: infer T }
    ? PropType<T>
    : never
}

/**
 * 简化的 props builder。
 * 用法：
 *   const props = defineProps(buildProps({ size: { default: 'md' } }))
 */
export function buildProps<O extends Record<string, unknown>>(
  propsDef: O,
): O & PublicPropDefinitions<O> {
  return propsDef as O & PublicPropDefinitions<O>
}

/**
 * 类型推导：把 buildProps 的输出提取出 props 类型
 */
export type BuildPropsToProps<T> = ExtractPropTypes<T>

/* ============================================================
 * sizeOf — 尺寸映射辅助
 * ============================================================ */

/**
 * 给一个尺寸值（sm/md/lg）拿一组 CSS 变量。
 *
 * 用法：
 *   const sz = sizeOf(props.size, 'btn')
 *   <button :style="{ minHeight: sz.h }" />
 */
export function sizeOf(
  size: string,
  prefix: 'btn' | 'input' | 'textarea',
): ComputedRef<Record<string, string>> {
  return computed<Record<string, string>>(() => {
    const variant = size === 'lg' ? 'lg' : size === 'sm' ? 'sm' : 'md'
    const result: Record<string, string> = {}
    if (prefix === 'btn') {
      result.h = `var(--ip-btn-h-${variant})`
      result.px = `var(--ip-btn-px-${variant})`
      result.py = `var(--ip-btn-py-${variant})`
      result.fs = `var(--ip-btn-fs-${variant})`
    } else if (prefix === 'input') {
      result.h = `var(--ip-input-h-${variant})`
      result.px = `var(--ip-input-px-${variant})`
      result.py = `var(--ip-input-py-${variant})`
    } else {
      result.minH = `var(--ip-textarea-min-h-${variant})`
      result.maxH = `var(--ip-textarea-max-h-${variant})`
    }
    return result
  })
}

/* ============================================================
 * isEmpty — 兜底空值判定
 * ============================================================ */

export function isEmpty(value: unknown): boolean {
  if (value === null || value === undefined) return true
  if (typeof value === 'string') return value.trim() === ''
  if (Array.isArray(value)) return value.length === 0
  return false
}

/* ============================================================
 * generateId — 简易 ID 生成器（用于 a11y）
 * ============================================================ */

let idCounter = 0

export function generateId(prefix = 'ip'): string {
  idCounter += 1
  return `${prefix}-${idCounter}-${Math.random().toString(36).slice(2, 7)}`
}

/* ============================================================
 * useStyleVars — 把对象挂到 :style 上
 * ============================================================ */

export function useStyleVars(record: Record<string, string>): CSSProperties {
  const style: Record<string, string> = {}
  for (const [k, v] of Object.entries(record)) {
    style[`--${k}`] = v
  }
  return style as CSSProperties
}
