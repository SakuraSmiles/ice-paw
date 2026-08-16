# @ice-paw/ui

IcePaw 设计系统与 Vue 3 组件库。

## 开发

本包是纯 CSS Tokens 设计系统库（无 Vue 组件、无独立编译产物），不提供 `dev` / `build` 流程，也无独立 `test` 脚本。

使用方式：在 `packages/ui/src/styles` 下编写或调整 CSS tokens，被 `@ice-paw/app` 通过包引用消费，无需额外构建步骤。修改后由 `@ice-paw/app` 的 Vite 流程自动带入。

## 使用

```ts
// 全量注册
import IcePawUI from '@ice-paw/ui/full'
import '@ice-paw/ui/styles'
app.use(IcePawUI)

// 按需引入
import { Button, Modal } from '@ice-paw/ui'
```
