/**
 * @ice-paw/ui 主入口
 *
 * 阶段 2：导出 P0 原子组件 + composables
 *  - Button / Input / Textarea / MessageBubble / Modal / Toast
 *  - IpFlex / IpContainer(布局组件，规范 icepaw-layout-system.md v1.1)
 *  - useToast / provideToast
 *  - 类型定义
 */

import './styles/index.css'

/* 组件 */
export { Button } from './components/button'
export type {
  ButtonProps,
  ButtonEmits,
  ButtonVariant,
  ButtonSize,
} from './components/button/types'

export { Input } from './components/input'
export type {
  InputProps,
  InputEmits,
  InputSize,
  InputState,
} from './components/input/types'

export { Textarea } from './components/textarea'
export type {
  TextareaProps,
  TextareaEmits,
  TextareaSize,
} from './components/textarea/types'

export { MessageBubble } from './components/message-bubble'
export type {
  MessageBubbleProps,
  MessageBubbleEmits,
  MessageRole,
} from './components/message-bubble/types'

export { Modal } from './components/modal'
export type {
  ModalProps,
  ModalEmits,
  ModalSize,
} from './components/modal/types'

export {
  Toast,
  ToastContainer,
  useToast,
  provideToast,
  createToastApi,
  ToastApiKey,
} from './components/toast'
export type {
  ToastType,
  ToastPosition,
  ToastOptions,
  ToastInstance,
  ToastApi,
} from './components/toast/types'

/* 布局组件（规范 icepaw-layout-system.md v1.1 P0） */
export { Flex as IpFlex } from './components/flex'
export type {
  FlexProps as IpFlexProps,
  FlexDirection,
  FlexAlign,
  FlexJustify,
  FlexWrap,
  FlexSeparator,
  SpaceSize,
  SizeProp,
} from './components/flex/types'

export { Container as IpContainer } from './components/container'
export type {
  ContainerProps as IpContainerProps,
  ContainerMaxWidth,
  ContainerPadding,
} from './components/container/types'

/* 工具 */
export {
  sizeOf,
  isEmpty,
  generateId,
  useStyleVars,
} from './utils'

/* 工具类型 */
export type { BuildPropsToProps } from './utils'

/* composables */
export { useToast as useToastComposable } from './components/toast/useToast'
