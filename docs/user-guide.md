# 使用指南

## 安装

从 [Releases](https://github.com/your-org/ice-paw/releases) 页面下载安装包。Windows 使用 `.msi`，macOS 使用 `.dmg`，Linux 使用 `.AppImage`。

首次启动时，应用会自动在本地创建 SQLite 数据库和 Stronghold 加密 vault，无需注册或联网。

## 配置 API Key

IcePaw 不内置任何 API Key，需要自行提供。打开设置（右上角齿轮图标）→ Agents → 新建 Agent：

1. 选择 Provider（OpenAI、Anthropic、智谱 GLM、DeepSeek、MiniMax）
2. 填写 API Key
3. 选择模型，填写 Agent 名称和 ID
4. 点击创建

API Key 存储在 Stronghold vault 中，不会明文写入数据库。Key 仅在与 LLM Provider 通信时使用，不会发送给任何第三方服务器。

如果使用中转 API 或兼容端点，在创建 Agent 时填写 `base_url` 即可，留空则使用 Provider 默认地址。

## Agent

Agent 是一组配置的集合：Provider、模型、System Prompt、温度参数等。同一个 Agent 可以开启多个对话，对话之间互不影响。

### 使用 agent.yaml

如果为 Agent 设置了 workspace 路径，可以在该目录下放置 `agent.yaml`，应用会自动读取：

```yaml
system_prompt: "你是一个 Rust 代码审查助手..."
temperature: 0.3
```

修改文件后即时生效，无需重启。Agent 设置页上会显示绿色标签提示已读取 `agent.yaml`。

### 编辑与删除

点击 Agent 卡片展开编辑面板，可以修改 Provider、模型、API Key、`base_url` 等配置。删除 Agent 时，已有的对话记录不会丢失。

## 对话

侧栏点击「新建对话」并选择 Agent 即可开始。回复采用流式输出，逐字渲染。

当 Agent 调用工具时（如读文件、执行命令），对话中会显示调用卡片，包含工具名、参数和执行结果，点击可以展开查看详情。

附带的图片会转为 base64 编码发送给模型。不支持图片理解的模型会自动隐藏图片附件。

### 会话操作

- **重命名**：双击标题进入编辑模式，Enter 确认，Escape 取消
- **置顶**：侧栏会话项右侧菜单 → 置顶
- **删除**：侧栏会话项右侧菜单 → 删除（不可恢复）
- **搜索**：侧栏顶部搜索框，输入标题关键词实时过滤

### 切换会话

如果在某个会话中正在生成回复，切换到其他会话再切回来，已生成的部分内容会被保留（bgStreams 快照机制），不需要等生成完成才能切走。

## 项目空间

项目空间用于将相关的 Agent 和会话归类管理。例如将「工作」和「个人项目」分别建立 project，切换时各自独立，互不干扰。

### 创建与配置

侧栏顶部下拉 → 管理项目 → 新建项目。填写名称、描述，选择初始成员 Agent。可以指定 workspace 路径，该路径会作为当前项目下文件工具的默认工作目录。

### 归档与删除

暂时不用的项目可以归档——从活跃列表中收起，内部会话保持完整。需要时恢复即可。

永久删除时会弹窗确认：选择「连同会话删除」则对话全部移除；选择「仅删除项目」则对话变为散落状态（无项目归属，但数据保留）。

## 工具（MCP）

Agent 可以调用两类工具：内置工具和外部 MCP Server 接入的工具。

### 内置工具

以下工具开箱即用：

| 工具 | 功能 |
|------|------|
| `read_file` / `write_file` / `edit_file` | 文件读写和精确替换 |
| `list_directory` | 列出目录内容 |
| `search_files` | 正则搜索文件内容（基于 ripgrep） |
| `run_command` | 执行 Shell 命令（每次调用弹窗确认） |
| `git` | 只读 Git 操作：status / diff / log / show |
| `web_fetch` | 抓取网页内容并转为 Markdown |
| `search_kb` / `read_kb_document` | 搜索和读取知识库文档 |
| `read_agent_config` | 读取 Agent 自身的 agent.yaml 配置 |

### 权限分级

工具调用采用三级权限模型：

- **Always**：安全只读操作，不需要用户确认（如 `git status`、`web_fetch`）
- **Confirm**：需要用户逐次批准的操作（如 `run_command`）
- **PathWhitelist**：限定在工作区路径内的文件操作

### 接入外部 MCP Server

如果现有工具不满足需求，可以在设置 → Tools (MCP) 中添加外部 MCP Server。填写启动命令（如 `npx -y @anthropic/mcp-server-postgres`）、参数、环境变量（如数据库连接串），并选择 scope：

- `global`：所有 Agent 共享
- `per_agent`：仅指定 Agent 可用

信任级别设为 `trusted` 则跳过确认弹窗，`untrusted` 则每次调用都需批准。外部 Server 子进程的环境变量经过白名单过滤，不会泄漏本机的 API Key。

## 知识库

将本地文档目录绑定为知识库后，Agent 可以通过 `search_kb` 搜索相关内容并在对话中引用。

知识库按 scope 分为三级：

- **全局**：设置 → Knowledge Base 中配置
- **Agent 级**：`<agent_workspace>/kb/` 目录
- **项目级**：`<project_workspace>/kb/` 目录

支持的文件格式包括 `.md`、`.txt`、`.json` 等。放入文件后会自动索引，修改或删除文件后索引同步更新。如果配置了 embedding 模型，检索时按语义相似度排序。需要强制重建索引时，可以在设置页点 Reindex。

## 设置

右上角齿轮图标打开设置页，共五个 Tab：

- **General**：默认 workspace 路径、时区（点 Detect 自动检测）、数据目录（点文件夹图标打开）、主题、字体大小、键盘快捷键
- **Agents**：管理 Agent 的创建、编辑和删除
- **Tools (MCP)**：管理内置工具和外部 MCP Server
- **Knowledge Base**：管理全局知识库
- **Logs**：查看运行日志（按天轮转，排查网络错误、工具执行失败、MCP 连接异常等）

## 数据目录

各平台的应用数据目录路径：

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\com.icepaw.app\` |
| macOS | `~/Library/Application Support/com.icepaw.app/` |
| Linux | `~/.local/share/com.icepaw.app/` |

目录下的主要文件：

- `ice-paw.db` — 对话记录、Agent 配置、项目信息（SQLite）
- `stronghold.hold` — API Key 加密 vault
- `logs/` — 运行日志

换设备时，将整个目录复制到新设备对应路径即可迁移全部数据。
