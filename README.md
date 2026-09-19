# IcePaw

> 本地优先的 LLM 对话工作站。多 Agent、工具调用、知识库、委派协作——数据全部留在你的机器上。

[![CI](https://github.com/SakuraSmiles/ice-paw/actions/workflows/ci.yml/badge.svg)](https://github.com/SakuraSmiles/ice-paw/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Version](https://img.shields.io/badge/version-0.9.2-blue)](https://github.com/SakuraSmiles/ice-paw/releases)

## 简介

IcePaw 是一个桌面 AI 对话工作站：同时配置多个 Agent，各自绑定不同的 LLM Provider（OpenAI / Anthropic / 智谱 / DeepSeek / MiniMax 等），在同一个窗口中协作使用。

Agent 可以执行 Shell 命令、读写文件、检索知识库，还能把任务**委派给其他 Agent**——每次委派都是完整会话，全程可审计。所有对话记录、Agent 配置存在本地 SQLite，API Key 经 Stronghold 加密落盘；不上传、不订阅、不限速，字体等资源全部离线内置，断网首启观感一致。

## 截图

![浅色主题聊天页](docs/screenshots/chat-light.png)

| 深色主题 | 会话轨迹 |
|---|---|
| ![深色主题聊天页](docs/screenshots/chat-dark.png) | ![会话轨迹页](docs/screenshots/trajectory.png) |

## 功能

- **多 Agent 协作**：主 Agent 可把任务委派给专家 Agent（委派=完整子会话，轨迹可回放）；跨会话信箱异步互投消息；任务面板实时查看进行中的委派
- **频道群聊**：项目成员共用一条串行共享流——@ 点名接力、统筹者编排广播、护栏截断风暴
- **生成中插话（Steer）**：agent 正在生成时直接发送——在途回复在工具边界干净截断，插话排队、静默聚合后自动续跑
- **Word 文档能力**：读侧结构投影 + zip 手术引擎编辑（格式/表格/样式），模板优先生成整篇文档 + 生成自检
- **屏幕读写**：截图 / 点击 / 键鼠操作 + 屏幕共享通道（一次授权会话内免逐次弹卡，物理输入优先让路）
- **Agent 代配置**：在对话中直接让 Agent 帮你创建或修改 Agent——提案卡片审批，Agent 全程无写权限
- **工具调用**：Shell 命令、文件读写、Git 操作、正则搜索代码、抓取网页
- **MCP 扩展**：接入外部 MCP Server（stdio / streamable HTTP），按 Agent 收窄工具面或整 Server 会话级信任
- **知识库**：本地文档自动索引，语义搜索检索；内置产品帮助文档
- **项目空间**：按项目归类会话与 Agent；项目轨迹页跨会话回看全量事件流
- **无限续写**：预算提醒 + 触顶收尾 + 摘要失败确定性折叠——长任务不再被神秘打断
- **会话事件日志**：每次对话是可回放的事件流，支持导出与审计
- **本地优先**：数据本地存储，无需注册账号；字体离线内置，无需联网

## 安装

从 [Releases](https://github.com/SakuraSmiles/ice-paw/releases) 页面下载 Windows 安装包（NSIS `.exe`，内置离线 WebView2 运行时，免管理员安装）。macOS（Apple Silicon）可从源码构建。

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

各子系统的设计真相源（Steer / 频道 / 工具权限 / 屏幕读写 / Word 能力路线等）见 [docs/](docs/) 目录。

## FAQ

**支持哪些 Provider？**  
OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax，以及任何兼容 OpenAI 或 Anthropic 格式的 API 端点。

**数据怎么迁移？**  
将 `ice-paw.db`、`stronghold.hold` 和 `images/` 目录（外置图片内容）一起复制到新设备的应用数据目录即可。各平台路径见[使用指南](docs/user-guide.md)。

**遇到问题怎么反馈？**  
[提交 Issue](https://github.com/SakuraSmiles/ice-paw/issues)。

---

MIT © IcePaw Contributors
