# IcePaw

> 桌面 AI 聊天客户端，数据本地存储，API Key 加密保管。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust tests](https://img.shields.io/badge/Rust%20tests-420%20passed-success)](https://github.com/your-org/ice-paw)
[![Frontend tests](https://img.shields.io/badge/Frontend%20tests-31%20passed-success)](https://github.com/your-org/ice-paw)

## 简介

IcePaw 是一个桌面 AI 聊天工具，支持同时配置多个 Agent，每个 Agent 可以绑定不同的 LLM Provider（OpenAI / Anthropic / 智谱 / DeepSeek / MiniMax 等），在同一个窗口中切换使用。

所有对话记录、Agent 配置都存在本地 SQLite 中，API Key 经 Stronghold 加密后落盘，不上传、不订阅、不限速。代码开源，可以自行构建或审计。

## 截图

<!-- TODO: 截图占位 -->

## 功能

- **多 Agent 管理**：每个 Agent 独立配置 Provider、模型、System Prompt、温度参数、工具权限
- **会话管理**：置顶、重命名、删除、搜索，按项目空间归类
- **工具调用**：Agent 可以执行 Shell 命令、读写文件、Git 操作、正则搜索代码、抓取网页
- **MCP 扩展**：支持接入外部 MCP Server（stdio），可按 Agent 或全局维度配置
- **知识库**：将本地文档目录绑定为知识库，Agent 可通过语义搜索检索内容
- **项目空间**：按项目归类 Agent 和会话，支持归档与恢复
- **本地优先**：所有数据存本地，无需注册账号，无需联网

## 安装

从 [Releases](https://github.com/your-org/ice-paw/releases) 页面下载对应平台的安装包。

首次打开后，到设置 → Agents → 新建 Agent，填入 Provider 和 API Key 即可开始对话。

## 文档

- [使用指南](docs/user-guide.md)
- [架构文档](docs/architecture.md)
- [贡献指南](CONTRIBUTING.md)

## FAQ

**支持哪些 Provider？**  
OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax，以及任何兼容 OpenAI 或 Anthropic 格式的 API 端点。

**数据怎么迁移？**  
将 `ice-paw.db` 和 `stronghold.hold` 复制到新设备的应用数据目录即可。各平台路径见[使用指南](docs/user-guide.md)。

**遇到问题怎么反馈？**  
[提交 Issue](https://github.com/your-org/ice-paw/issues)。

---

MIT © IcePaw Contributors
