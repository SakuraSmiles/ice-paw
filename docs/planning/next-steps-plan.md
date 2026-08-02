---
name: next-steps-plan
description: 前端功能开发剩余任务的拆分计划
metadata:
  type: project
---

# IcePaw 前端开发计划（P0 → P1）

## Phase 0-1：状态管理层（1 次会话）
先搭建 Pinia store，所有 API 调用统一管理，后面的步骤直接消费 store。

- [ ] 创建 `stores/chat.ts` — 会话列表、当前会话、消息列表状态
- [ ] 创建 `stores/agent.ts` — Agent 列表状态（已有数据）
- [ ] 接入 `bridge.conversations.list()` 加载真实会话

## Phase 0-2：新建对话 + 会话切换（1 次会话）
聊天页面真正能运行的核心链路。

- [ ] 侧栏"新建对话"→ 弹出 Agent 选择器 → 调 `create_conversation` → 进入聊天
- [ ] 侧栏会话列表加载真实数据，点击切换加载对应消息
- [ ] 聊天头部显示当前 Agent 名

## Phase 0-3：消息收发（1-2 次会话）
流式聊天的完整实现，这是最复杂的部分。

- [ ] `ChatInput` 接入 `bridge.chat.sendMessage()`
- [ ] 监听 Tauri events：`chat:start` / `chat:chunk` / `chat:done` / `chat:error`
- [ ] 消息实时追加到列表，流式文字逐步展示
- [ ] 接入 `bridge.messages.list()` 加载历史消息（含分页）
- [ ] 加载中骨架屏

## Phase 1-1：会话管理（1 次会话）
- [ ] 会话删除（确认弹窗 + 调后端）
- [ ] 会话重命名（双击/右键 → inline rename）
- [ ] 会话置顶

## Phase 1-2：Agent 选择切换（1 次会话）
- [ ] 聊天头部点击 Agent 名 → 弹出 Agent 列表 → 切换
- [ ] 切换后加载该 Agent 的会话列表

## Phase 1-3：体验增强（1 次会话）
- [ ] 消息 hover 复制按钮
- [ ] 输入框自动增高（auto-resize textarea）
- [ ] 搜索框过滤会话

---

**共约 7-9 次会话**，每次约 30-60 分钟。
