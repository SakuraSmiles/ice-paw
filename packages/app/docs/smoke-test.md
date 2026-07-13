# IcePaw 冒烟测试 Checklist

> 手动真机冒烟测试指南，覆盖 OpenAI / GLM / MiniMax / Claude 四家 provider。

## 前置准备

1. 在 Windows 真机上启动 IcePaw（需要梯子访问外网 API）
2. 准备好各 provider 的 API Key
3. 创建 Agent（按下方各 provider 配置）

## 各 Provider 测试配置

### OpenAI

| 配置项 | 值 |
|--------|-----|
| Provider | `openai` |
| Model | `gpt-4o` |
| Base URL | `https://api.openai.com` |
| System Prompt | `You are a helpful assistant.` |
| Temperature | 0.7 |
| Max Tokens | 4096 |

### GLM（智谱）

| 配置项 | 值 |
|--------|-----|
| Provider | `glm` |
| Model | `glm-4-flash` |
| Base URL | `https://open.bigmodel.cn/api/paas/v4` |
| System Prompt | `你是一个有用的助手。` |

### MiniMax

| 配置项 | 值 |
|--------|-----|
| Provider | `minimax` |
| Model | `MiniMax-M2.5` |
| Base URL | `https://api.minimaxi.com/anthropic` |
| System Prompt | `你是一个有用的助手。` |

### Claude（Anthropic）

| 配置项 | 值 |
|--------|-----|
| Provider | `anthropic` |
| Model | `claude-sonnet-4-20250514` |
| Base URL | `https://api.anthropic.com` |
| System Prompt | `You are a helpful assistant.` |
| Temperature | 0.7 |
| Max Tokens | 4096 |

## 测试步骤

### 1. 创建 Agent

- [ ] 进入 Agent 管理页面
- [ ] 按上方配置创建 Agent（每个 provider 一个）
- [ ] 验证创建成功，列表显示正确

### 2. 单轮对话

- [ ] 选择 Agent，创建新会话
- [ ] 发送消息：「你好，请用一句话介绍你自己」
- [ ] 验证流式返回：文字逐步显示，无卡顿
- [ ] 验证最终内容完整（无截断）

### 3. 多轮对话

- [ ] 继续发送：「上一条消息你说了什么？」
- [ ] 验证 LLM 能引用上下文（证明历史消息正确传递）
- [ ] 再发一条确认连贯性

### 4. 长文本输出

- [ ] 发送：「请生成一个完整的 Rust HTTP server 实现，包含完整的错误处理和测试」
- [ ] 验证长输出流式渲染流畅（无卡顿/批处理现象）
- [ ] 验证完整输出（无中途截断）

### 5. 取消生成

- [ ] 发送一个会触发长回复的问题
- [ ] 在流式输出过程中点击「停止」按钮
- [ ] 验证生成立即停止
- [ ] 验证已输出的内容保留在消息中
- [ ] 验证可以继续发送新消息（状态正确恢复）

### 6. 错误处理

- [ ] 创建一个使用无效 API Key 的 Agent
- [ ] 发送消息
- [ ] 验证显示错误提示（非白屏/崩溃）
- [ ] 验证错误消息包含有意义的描述

### 7. 断网恢复（重试机制）

- [ ] 发送消息
- [ ] 在流式输出过程中断开网络（拔网线 / 关 WiFi）
- [ ] 等待 3~5 秒后恢复网络
- [ ] 验证显示「正在重新连接...」提示
- [ ] 验证恢复后内容衔接（已收到内容保留）
- [ ] 验证重试耗尽后显示友好错误

## 记录模板

| Provider | 创建 Agent | 单轮对话 | 多轮对话 | 长文本 | 取消 | 错误 | 重试 | 备注 |
|----------|-----------|---------|---------|--------|------|------|------|------|
| OpenAI  | ☐         | ☐       | ☐       | ☐      | ☐    | ☐    | ☐    | |
| GLM     | ☐         | ☐       | ☐       | ☐      | ☐    | ☐    | ☐    | |
| MiniMax | ☐         | ☐       | ☐       | ☐      | ☐    | ☐    | ☐    | |
| Claude  | ☐         | ☐       | ☐       | ☐      | ☐    | ☐    | ☐    | |

## 常见问题

**Q: MiniMax 连接超时**
A: 确保梯子已开启，MiniMax API 域名 `api.minimaxi.com` 需要代理访问。

**Q: GLM 返回 404**
A: 检查 base_url 是否正确。GLM coding 用户的 base_url 为 `https://open.bigmodel.cn/api/coding/paas/v4`。

**Q: Claude 返回 401**
A: 确认 Anthropic API Key 有效，且未过期。
