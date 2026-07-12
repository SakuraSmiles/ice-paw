/**
 * Button — 公开 Props / Emits / 类型定义
 *
 * 规范：icepaw-design-system.md §2.1
 * 变体：primary / secondary / ghost / danger
 * 尺寸：sm / md / lg
 * 状态：default / hover / focus / active / disabled / loading
 */

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger'
export type ButtonSize = 'sm' | 'md' | 'lg'
export type ButtonType = 'button' | 'submit' | 'reset'

export interface ButtonProps {
  /** 视觉变体 */
  variant?: ButtonVariant
  /** 尺寸 */
  size?: ButtonSize
  /** 加载中：自动 disabled + 显示 spinner（v1.0.1 增补） */
  loading?: boolean
  /** 手动 disabled（loading=true 时自动覆盖） */
  disabled?: boolean
  /** 是否块级（width: 100%） */
  block?: boolean
  /** 仅图标按钮（方形 width = height） */
  iconOnly?: boolean
  /** 原生 button type */
  type?: ButtonType
}

export interface ButtonEmits {
  /** 点击事件（loading/disabled 时不触发） */
  (e: 'click', ev: MouseEvent): void
}

/* 推导实际禁用态：loading 时强制 disabled */
export type EffectiveButtonState = 'default' | 'disabled' | 'loading'
