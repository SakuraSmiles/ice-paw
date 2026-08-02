# IcePaw

> 桌面端 AI 聊天客户端 —— 数据本地存储，隐私自主可控

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust tests](https://img.shields.io/badge/Rust%20tests-420%20passed-success)](https://github.com/your-org/ice-paw)
[![Frontend tests](https://img.shields.io/badge/Frontend%20tests-31%20passed-success)](https://github.com/your-org/ice-paw)

IcePaw 是一款桌面 AI 助手。你可以配置多个 AI Agent（支持 OpenAI / Anthropic / 智谱 GLM 等），在同一界面中切换对话。所有数据存在本地，API Key 加密保管，不会上传到任何服务器。

## 为什么选择 IcePaw？

不同于云端 AI 服务，IcePaw 把控制权交还给你 —— 对话数据完全本地存储，API Key 端到端加密，无需订阅、无需注册、永不限速。你的数据你做主，代码完全开源，任何人都可以审计或贡献。

## 特性

- 🤖 **多 Agent**：每个 Agent 独立配置模型、系统提示词、温度参数
- 💬 **多会话**：置顶、重命名、搜索，项目空间归类管理
- 🔧 **工具调用**：Agent 可执行 shell 命令、读写文件、git 操作、搜索代码、抓取网页
- 🔒 **本地优先**：对话历史存 SQLite，API Key 经 Stronghold 加密，你拥有全部数据
- 💻 **跨平台**：Windows / macOS / Linux

## 安装

下载最新版本：[Releases](https://github.com/your-org/ice-paw/releases)

| 平台 | 安装包 |
|------|--------|
| Windows | `.msi` 或 `.exe` |
| macOS | `.dmg` |
| Linux | `.AppImage` 或 `.deb` |

## 快速上手

### 1. 添加 API Key

打开 IcePaw → 设置 → Agent 管理 → 新建 Agent，填写你的 LLM Provider API Key（如 OpenAI / Anthropic）。

### 2. 开始对话

侧栏点击「新建对话」，选择 Agent，输入消息即可。

### 3. 配置工具（可选）

在设置 → MCP Server 中可以添加外部工具（如 npx 包），扩展 Agent 的能力。

## 数据与隐私

所有数据存储在本地应用数据目录：

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\com.icepaw.app\` |
| macOS | `~/Library/Application Support/com.icepaw.app/` |
| Linux | `~/.local/share/com.icepaw.app/` |

- `ice-paw.db` — 对话历史、Agent 配置
- `stronghold.hold` — API Key 加密 vault

API Key **永远不会**以明文写入数据库或发送到第三方服务器。通信仅在你的设备和 LLM Provider 之间进行。

## 常见问题

**支持哪些 LLM Provider？**  
OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax，以及任何 OpenAI/Anthropic 兼容 API。

**数据可以迁移吗？**  
可以。直接复制 `ice-paw.db` 和 `stronghold.hold` 到新设备的对应目录即可。

**如何反馈问题？**  
[GitHub Issues](https://github.com/your-org/ice-paw/issues)

## 文档

- [用户指南](user-guide.md)
- [架构文档](architecture.md)
- [贡献指南](CONTRIBUTING.md)

---

## License

MIT © IcePaw Contributors
