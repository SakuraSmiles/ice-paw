/**
 * Avatar — 公开 Props / Emits / 类型定义
 *
 * 规范：icepaw-p0-component-specs.md §一
 * 替代：app/components/common/AgentAvatar.vue（迁移后删除 app 层副本）
 */

import type { Component } from 'vue'

/** 形状：rounded=8px 方角（默认，与 AgentAvatar、消息气泡统一）；circle=正圆（仅用户头像） */
export type AvatarShape = 'circle' | 'rounded'

/** 尺寸梯度：xs=20 / sm=28 / md=36 / lg=48 / xl=64 / xxl=96 */
export type AvatarSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl' | 'xxl'

/**
 * REQ-UI-008A：在线状态指示。
 * online  = 绿点；offline = 灰点；busy = 红点
 * 不传时根节点不渲染任何状态指示点。
 */
export type AvatarStatus = 'online' | 'offline' | 'busy'

/**
 * 内容源（受控）。
 * 由父组件决定渲染哪种模式，本组件只根据 type 渲染对应结构。
 */
export type AvatarSource =
  | { type: 'image'; src: string; alt?: string }
  | { type: 'icon'; icon: Component; color?: string }
  | { type: 'initials'; text: string; bgColor: string; fgColor?: string }
  | { type: 'default'; icon?: Component; fallbackIcon?: Component }

/** 文件校验结果（emit 时携带） */
export interface AvatarUploadError {
  /** 错误码 */
  code: 'file_too_large' | 'invalid_mime' | 'no_file'
  /** 错误信息（已本地化为中文，可直接展示） */
  message: string
}

export interface AvatarProps {
  /** 内容源（必填）。type=image 时 src 支持 http(s) URL 或 data: URL。 */
  source: AvatarSource

  /** 尺寸。默认 'md' (36px) */
  size?: AvatarSize

  /** 形状。默认 'rounded' (8px) */
  shape?: AvatarShape

  /** 可点击上传模式。true 时：hover 显示蒙层 + Camera 图标，点击触发隐藏 <input type="file"> */
  uploadable?: boolean

  /** uploadable 时 input accept。默认 'image/png,image/jpeg,image/gif,image/webp' */
  accept?: string

  /** uploadable 时最大字节数。默认 2 * 1024 * 1024 (2MB) */
  maxSize?: number

  /** uploadable 且 source.type === 'image' 时，是否显示移除按钮（hover 出现 ✕） */
  removable?: boolean

  /** 上传中（覆盖在头像上的 spinner） */
  loading?: boolean

  /** 禁用（不可点击 / 不可上传 / 灰态） */
  disabled?: boolean

  /** 自定义根节点 alt（image 模式时优先取 source.alt，无则用此） */
  alt?: string

  /** 自定义根节点 aria-label（默认根据 source.type 推断） */
  ariaLabel?: string

  /**
   * REQ-UI-008：图片加载失败（onerror）回退到的名称。
   * 仅对 source.type='image' 生效：图片 URL 加载失败时，自动回退为使用 `name` 首字符渲染的文字头像。
   */
  name?: string

  /**
   * REQ-UI-008A：头像在线状态指示点。
   * 'online' | 'offline' | 'busy'，不传时根节点不渲染指示点。
   */
  status?: AvatarStatus
}

export interface AvatarEmits {
  /** uploadable=true 时选择文件后触发（已通过大小 / MIME 校验），由父组件处理上传 */
  (e: 'upload', file: File): void

  /** uploadable=true 时校验失败 */
  (e: 'upload-error', error: AvatarUploadError): void

  /** removable=true 时点击 ✕ 触发，父组件应将 source 切回 default */
  (e: 'remove'): void

  /** 点击事件（disabled / loading 时不触发） */
  (e: 'click', ev: MouseEvent): void

  /** uploadable hover / focus 状态（用于内部覆盖层显示控制，外部一般不用） */
  (e: 'hover', hovered: boolean): void

  /** REQ-UI-008：图片加载失败时触发 */
  (e: 'error', payload: { code: 'load_failed'; message: string; fallback: true }): void
}
