/**
 * Input — 公开 Props / Emits
 *
 * 规范：icepaw-design-system.md §2.2
 */

export type InputSize = 'sm' | 'md' | 'lg'
export type InputState = 'default' | 'disabled' | 'error' | 'readonly'

export interface InputProps {
  /** v-model */
  modelValue?: string | number | null
  /** 尺寸 */
  size?: InputSize
  /** 占位符 */
  placeholder?: string
  /** 错误态（红边 + danger focus ring） */
  error?: boolean
  /** 错误信息（用于 v1.0.1 错误提示（仅 color 不够，配文字）） */
  errorMessage?: string
  /** 禁用 */
  disabled?: boolean
  /** 只读 */
  readonly?: boolean
  /** 一键清空按钮（X），右侧出现 */
  clearable?: boolean
  /** HTML input type */
  type?: string
  /** HTML input name */
  name?: string
  /** HTML input autocomplete */
  autocomplete?: string
  /** 关联 label id（a11y） */
  inputId?: string
  /** 最大长度 */
  maxlength?: number | string
  /** 表单 field set（用 <fieldset disabled> 时也禁用） */
}

export interface InputEmits {
  (e: 'update:modelValue', value: string): void
  (e: 'focus', ev: FocusEvent): void
  (e: 'blur', ev: FocusEvent): void
  (e: 'clear'): void
  (e: 'enter', ev: KeyboardEvent): void
}
