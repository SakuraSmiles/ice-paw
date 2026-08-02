# IcePaw

> 桌面 AI 聊天客户端。数据在你本地，Key 在你手里。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust tests](https://img.shields.io/badge/Rust%20tests-420%20passed-success)](https://github.com/your-org/ice-paw)
[![Frontend tests](https://img.shields.io/badge/Frontend%20tests-31%20passed-success)](https://github.com/your-org/ice-paw)

就是一个桌面版的 AI 聊天工具。你可以配多个 Agent（OpenAI、Anthropic、智谱、DeepSeek、MiniMax 都支持），在同一个窗口里切换对话。所有东西存本地，API Key 加密保管，不上传、不订阅、不限速。

## 截图

<!-- TODO: 截图占位-->

## 能做什么

- 🤖 **多 Agent** — 每个 Agent 独立配模型、system prompt、温度
- 💬 **多会话** — 置顶、重命名、搜索，项目空间归类
- 🔧 **调工具** — Agent 能执行 shell 命令、读写文件、git 操作、搜代码、抓网页
- 🔒 **本地优先** — SQLite 存对话、Stronghold 加密 Key
- 💻 **跨平台** — Windows / macOS / Linux

## 装一个试试

去 [Releases](https://github.com/your-org/ice-paw/releases) 下载。

打开 → 设置 → Agents → 新建 Agent → 填 API Key → 开始聊天。

## 文档

- [使用指南](docs/user-guide.md) — 功能介绍和操作说明
- [架构文档](docs/architecture.md) — 系统设计和数据流
- [贡献指南](CONTRIBUTING.md) — 本地开发环境搭建

## 常见问题

**支持哪些 Provider？**  
OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax，以及任何兼容 OpenAI/Anthropic 格式的 API。

**数据怎么迁移？**  
把 `ice-paw.db` 和 `stronghold.hold` 拷到新设备的数据目录就行，路径见[使用指南](docs/user-guide.md#数据在哪)。

**有 bug 怎么办？**  
[提 Issue](https://github.com/your-org/ice-paw/issues)。

---

MIT © IcePaw Contributors
