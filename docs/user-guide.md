# 使用指南

## 安装

去 [Releases](https://github.com/your-org/ice-paw/releases) 下载对应平台的安装包。Windows 用 `.msi`，macOS 用 `.dmg`，Linux 用 `.AppImage`。

装完打开就行。第一次启动会自动建数据库和加密 vault，不需要注册账号、不需要联网验证，什么都没有。数据全在你本机。

## 配 API Key

IcePaw 不内置任何 API Key，你得用自己的。

打开设置（右上角齿轮）→ Agents → 点「新建 Agent」：

1. 选 Provider（OpenAI / Anthropic / 智谱 / DeepSeek / MiniMax）
2. 填入 API Key
3. 选模型，起个名字
4. 点创建

搞定。Key 存在 Stronghold vault 里，不会明文落盘。关于安全性：它只在你向 LLM Provider 发请求的时候才用，不会传给任何第三方。

另外，如果你用的是中转 API 或者兼容接口，填 `base_url` 就行，留空走默认地址。

## Agent

Agent 就是一组配置：用什么模型、什么 system prompt、温度调到多少。一个 Agent 可以开一堆对话，对话之间互不影响。

### 用 agent.yaml 管理行为

如果你给 Agent 设了 workspace 路径，在那个目录下放个 `agent.yaml`，IcePaw 会自动读：

```yaml
system_prompt: "你是一个 Rust 代码审查助手..."
temperature: 0.3
```

改了文件直接生效，不用重启。Agent 设置页上会有个绿色标签提示你「已读取 agent.yaml」。

### 编辑和删除

点 Agent 卡片展开编辑，可以改 provider、model、API Key。要删的话点右上角三个点 → 删除。删除后已有的对话还在，不会丢。

## 对话

侧栏点「新建对话」选 Agent 就能开始聊。回复是流式的——字一个一个往外蹦。

如果 Agent 调了工具（比如读文件、跑命令），对话里会显示调用卡片——哪个工具、什么参数、执行结果，点一下可以展开看详情。

### 管理对话

- **重命名**：在标题上双击就能改
- **置顶**：点会话右边的三个点 → 置顶
- **删除**：三个点 → 删除（不可恢复）
- **搜索**：侧栏顶部的搜索框，输入标题关键词实时过滤

### 切换对话不会丢

如果你在一个会话里正在生成回复，切到另一个会话再切回来——之前生成到一半的内容还在。这就是 bgStreams 做的快照，不用等它生成完才能切走。

## 项目空间

项目就是把几个相关 Agent 和会话归到一起。比如「工作项目」和「个人玩具」各建一个 project，切来切去互不干扰。

### 创建

侧栏顶部下拉 → 管理项目 → 新建项目。填名字、描述、选初始成员。workspace 路径可以指定一个目录，这样这个项目里的 Agent 用文件工具时会限定在这个目录下。

### 归档 vs 删除

暂时不用的项目可以归档——从活跃列表里收起来，但里面的对话全都完好。想用的时候恢复就行。

永久删除会弹窗让你确认：要不要连同对话一起删？选「是」对话全删，选「否」对话变成散落状态（没有项目归属但还在）。

## 工具（MCP）

Agent 能调的工具分两类：内置的，和外部接进来的。

### 内置工具

开箱即用，不需要配置：

| 工具 | 干啥的 |
|------|--------|
| `read_file` / `write_file` / `edit_file` | 读写文件 |
| `list_directory` | 看目录里有什么 |
| `search_files` | 正则搜代码 |
| `run_command` | 执行 shell 命令（每次会弹窗让你确认） |
| `git` | status / diff / log / show（只读） |
| `web_fetch` | 抓网页内容 |
| `search_kb` / `read_kb_document` | 搜知识库 |
| `read_agent_config` | 读自己的 agent.yaml |

### 权限分级

- **Always**：安全只读操作，不用确认（如 git status、web_fetch）
- **Confirm**：危险操作，弹窗让你逐条批准（如 run_command）
- **PathWhitelist**：限定在工作区目录内的文件操作

### 接外部 MCP Server

如果你要扩展工具——比如接数据库、Jira、自定义 API——在设置 → Tools (MCP) 里添加外部 Server。填好启动命令（如 `npx -y @modelcontextprotocol/server-postgres`）、环境变量（如连接串），选 scope（所有 Agent 共享 / 单个 Agent 专属）。

信任级别设成 trusted 就跳过确认，untrusted 就每次调工具弹窗。外部 Server 的环境变量会经过白名单过滤，不会把你系统的 API Key 漏过去。安全细节可以看架构文档里的 MCP 章节。

## 知识库

把文档放在 `kb/` 目录下，Agent 就能搜到。分三个级别：

- **全局**：设置 → 知识库
- **Agent 级**：`<agent_workspace>/kb/`
- **项目级**：`<project_workspace>/kb/`

放进去的文件会被自动索引（支持 `.md`、`.txt`、`.json` 等）。对话中 Agent 调 `search_kb` 搜相关文档，`read_kb_document` 读全文。如果用了支持 embedding 的模型，搜索是按语义相似度排序的。

改文件后索引会自动更新，不用手动操作。如果想强制重建，设置页有个 Reindex 按钮。

## 设置

右上角齿轮打开。五个 tab：

**General**：工作区默认路径、时区（点 Detect 自动检测）、数据目录（点文件夹图标打开）、主题、字体、键盘快捷键。

**Agents**：管理 Agent。

**Tools (MCP)**：管理工具。

**Knowledge Base**：管理全局知识库。

**Logs**：运行日志，排查问题用——网络错误、工具执行失败、MCP 连接异常都看这里。

## 数据在哪

Windows：`%APPDATA%\com.icepaw.app\`
macOS：`~/Library/Application Support/com.icepaw.app/`
Linux：`~/.local/share/com.icepaw.app/`

里面三个东西：

- `ice-paw.db` — 所有对话、Agent 配置、项目信息
- `stronghold.hold` — API Key 加密 vault
- `logs/` — 运行日志

换电脑直接把整个目录拷过去就行。
