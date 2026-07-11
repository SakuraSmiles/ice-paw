# @ice-paw/ui

IcePaw 设计系统与 Vue 3 组件库。

## 开发

```bash
# 启动预览站（端口 5173）
pnpm dev:ui

# 构建 lib 产物
pnpm build:ui

# 运行单测
pnpm test
```

## 使用

```ts
// 全量注册
import IcePawUI from '@ice-paw/ui/full'
import '@ice-paw/ui/styles'
app.use(IcePawUI)

// 按需引入
import { Button, Modal } from '@ice-paw/ui'
```
