/**
 * MessageBubble — 公开 Props / Emits
 *
 * 规范：icepaw-design-system.md §2.3
 * 角色：user / assistant / system
 */

export type MessageRole = 'user' | 'assistant' | 'system'

export interface MessageBubbleProps {
  /** 消息角色 */
  role: MessageRole
  /** 角色显示名（assistant 必填，user 可选） */
  name?: string
  /** 时间戳（元信息） */
  timestamp?: string
  /** 附加元信息（响应时长、token 数等） */
  meta?: string
  /** 是否处于流式输出（assistant 时显示光标） */
  streaming?: boolean
  /** 错误信息（错误态显示） */
  error?: string
  /** 头部头像 URL（assistant） */
  avatar?: string
}

export interface MessageBubbleEmits {
  (e: 'copy'): void
  (e: 'regenerate'): void
  (e: 'retry'): void
}
