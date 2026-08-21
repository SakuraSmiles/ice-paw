# IcePaw

> 本地优先的 LLM 对话工作站。多 Agent、工具调用、知识库、委派协作——数据全部留在你的机器上。

[![CI](https://github.com/SakuraSmiles/ice-paw/actions/workflows/ci.yml/badge.svg)](https://github.com/SakuraSmiles/ice-paw/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Version](https://img.shields.io/badge/version-0.4.0-blue)](https://github.com/SakuraSmiles/ice-paw/releases)

## 简介

IcePaw 是一个桌面 AI 对话工作站：同时配置多个 Agent，各自绑定不同的 LLM Provider（OpenAI / Anthropic / 智谱 / DeepSeek / MiniMax 等），在同一个窗口中协作使用。

Agent 可以执行 Shell 命令、读写文件、检索知识库，还能把任务**委派给其他 Agent**——每次委派都是完整会话，全程可审计。所有对话记录、Agent 配置存在本地 SQLite，API Key 经 Stronghold 加密落盘；不上传、不订阅、不限速，字体等资源全部离线内置，断网首启观感一致。

## 截图

<!-- 截图：浅色主题聊天页 / 深色主题 / 项目轨迹页（占位待替换，见下方说明） -->

## 功能

- **多 Agent 协作**：主 Agent 可把任务委派给专家 Agent（委派=完整子会话，轨迹可回放）；任务面板实时查看进行中的委派
- **Agent 代配置**：在对话中直接让 Agent 帮你创建或修改 Agent——提案卡片审批，Agent 全程无写权限
- **工具调用**：Shell 命令、文件读写、Git 操作、正则搜索代码、抓取网页
- **MCP 扩展**：接入外部 MCP Server（stdio），按 Agent 或全局维度配置
- **知识库**：本地文档自动索引，语义搜索检索；内置产品帮助文档
- **项目空间**：按项目归类会话与 Agent；项目轨迹页跨会话回看全量事件流
- **无限续写**：预算提醒 + 触顶收尾 + 摘要失败确定性折叠——长任务不再被神秘打断
- **会话事件日志**：每次对话是可回放的事件流，支持导出与审计
- **本地优先**：数据本地存储，无需注册账号；字体离线内置，无需联网

## 安装

从 [Releases](https://github.com/SakuraSmiles/ice-paw/releases) 页面下载对应平台的安装包（macOS Apple Silicon 提供 dmg；Windows 可从源码构建）。

首次打开后，在设置中创建你的第一个 Agent（选 Provider、填 API Key、选模型），然后直接开始对话；或者在对话中对它说「帮我创建一个写代码的 agent」——Agent 会提交提案卡片，你填 Key 点批准即可。

## 从源码构建

```bash
git clone https://github.com/SakuraSmiles/ice-paw.git
cd ice-paw
pnpm install
pnpm tauri:build   # 产物在 packages/app/src-tauri/target/release/bundle/
```

需要 Node.js 20+、pnpm、Rust 工具链；macOS 另需 `brew install libsodium`，Windows 的 sodium 路径见 [CONTRIBUTING](CONTRIBUTING.md)。

## 文档

- [使用指南](docs/user-guide.md)
- [架构文档](docs/architecture.md)
- [多 Agent 协作设计](docs/multi-agent-architecture.md)
- [贡献指南](CONTRIBUTING.md)

## FAQ

**支持哪些 Provider？**  
OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax，以及任何兼容 OpenAI 或 Anthropic 格式的 API 端点。

**数据怎么迁移？**  
将 `ice-paw.db` 和 `stronghold.hold` 复制到新设备的应用数据目录即可。各平台路径见[使用指南](docs/user-guide.md)。

**遇到问题怎么反馈？**  
[提交 Issue](https://github.com/SakuraSmiles/ice-paw/issues)。

---

MIT © IcePaw Contributors
