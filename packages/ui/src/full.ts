/**
 * @ice-paw/ui 全量注册入口
 *
 * 用法（Tauri app）：
 *   import { createApp } from 'vue'
 *   import App from './App.vue'
 *   import IcePawUI from '@ice-paw/ui/full'
 *
 *   const app = createApp(App)
 *   app.use(IcePawUI)
 *   app.mount('#app')
 */

import type { App } from 'vue'
import { Button } from './components/button'
import { Input } from './components/input'
import { Textarea } from './components/textarea'
import { MessageBubble } from './components/message-bubble'
import { Modal } from './components/modal'
import { Toast, ToastContainer } from './components/toast'
import { Avatar as IpAvatar } from './components/avatar'
import { Card as IpCard } from './components/card'
import { Select as IpSelect } from './components/select'
import { EmptyState as IpEmptyState } from './components/empty-state'
import { DropdownMenu as IpDropdownMenu } from './components/dropdown'
import { Popconfirm as IpPopconfirm } from './components/popconfirm'
import { Flex as IpFlex } from './components/flex'
import { Container as IpContainer } from './components/container'

import './styles/index.css'

const IcePawUI = {
  install(app: App): void {
    app.component('IpButton', Button)
    app.component('IpInput', Input)
    app.component('IpTextarea', Textarea)
    app.component('IpMessageBubble', MessageBubble)
    app.component('IpModal', Modal)
    app.component('IpToast', Toast)
    app.component('IpToastContainer', ToastContainer)
    app.component('IpAvatar', IpAvatar)
    app.component('IpCard', IpCard)
    app.component('IpSelect', IpSelect)
    app.component('IpEmptyState', IpEmptyState)
    app.component('IpDropdownMenu', IpDropdownMenu)
    app.component('IpPopconfirm', IpPopconfirm)
    app.component('IpFlex', IpFlex)
    app.component('IpContainer', IpContainer)
  },
}

export default IcePawUI
export { IcePawUI }
