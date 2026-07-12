/**
 * Textarea — 公开 Props / Emits
 *
 * 规范：icepaw-design-system.md §2.2.3 (v1.0.1 增补尺寸变体)
 */

export type TextareaSize = 'sm' | 'md' | 'lg'

export interface TextareaProps {
  /** v-model */
  modelValue?: string | null
  /** 尺寸变体 */
  size?: TextareaSize
  /** 占位符 */
  placeholder?: string
  /** 错误态 */
  error?: boolean
  /** 错误信息 */
  errorMessage?: string
  /** 禁用 */
  disabled?: boolean
  /** 只读 */
  readonly?: boolean
  /** 自动撑高（基于内容） */
  autoResize?: boolean
  /** 自定义 resize handle（v1.0 §3.4） */
  resizable?: boolean
  /** name */
  name?: string
  /** 最大长度 */
  maxlength?: number | string
  /** 行数（默认 4 / 6 / 8） */
  rows?: number
}

export interface TextareaEmits {
  (e: 'update:modelValue', value: string): void
  (e: 'focus', ev: FocusEvent): void
  (e: 'blur', ev: FocusEvent): void
  (e: 'enter', ev: KeyboardEvent): void
  (e: 'escape', ev: KeyboardEvent): void
}
