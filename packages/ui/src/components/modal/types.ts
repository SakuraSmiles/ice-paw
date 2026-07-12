/**
 * Modal — 公开 Props / Emits
 *
 * 规范：icepaw-design-system.md §2.5
 * 尺寸：sm(400) / md(560) / lg(720)
 * 嵌套 modal：--ip-z-modal-content + 10 × nestedLevel
 */

export type ModalSize = 'sm' | 'md' | 'lg'

export interface ModalProps {
  /** v-model 控制显示 */
  modelValue?: boolean
  /** 尺寸变体 */
  size?: ModalSize
  /** 标题（不传则不显示 header） */
  title?: string
  /** 点击遮罩关闭（v1.0.1 §2.5.4：危险操作可禁用） */
  closeOnOverlay?: boolean
  /** 按 Esc 关闭 */
  closeOnEsc?: boolean
  /** 是否显示关闭按钮 */
  showClose?: boolean
  /** 自定义嵌套层级（每层 +10 z-index） */
  nestedLevel?: number
  /** 自定义 z-index 起点 */
  zIndex?: number
}

export interface ModalEmits {
  (e: 'update:modelValue', value: boolean): void
  (e: 'open'): void
  (e: 'close'): void
}
