---
title: 配置 MCP 工具与授权
summary: 怎么启用工具、MCP server 是什么、怎么添加外部 server（stdio 本地进程 / streamable HTTP 远程）、global/per_agent scope 区别、信任级别与会话级授权档、断线自动重连、为什么工具调用会弹授权窗、workspace 内免授权。
tags: [工具, MCP, MCP server, 授权, 权限, 确认弹窗, scope, per_agent, global, 文件工具, run_command, 外部 server, 添加 server, 传输, stdio, http, streamable, 信任级别, UE, Unreal, 断线, 重连, this_server]
---

# 配置 MCP 工具与授权

工具让 agent 不只是聊天，还能读写文件、执行命令、操作 git 等。工具基于 MCP（Model Context Protocol）。

## 启用工具

会话里有「启用工具」开关。打开后，agent 在需要时会自动调用已启用的工具，并多轮执行直到完成任务。

可在 agent 配置里限定**工具白名单**（`enabled_tools`）：只给 agent 开放部分工具，其余禁用。

## MCP Server 与 scope

工具由 **MCP server** 提供。在 **设置 → MCP / 工具集** 管理 server。每个 server 有 **scope**：

- **global**：所有 agent 共享。
- **per_agent**：按 agent 绑定各自的工作区（每个 agent 的文件操作指向自己的 workspace）。

内置工具（文件读写、命令执行、git、知识库等）开箱即用；外部 server 按需添加。

## 添加外部 server

在 **设置 → MCP / 工具集** 点「添加 Server」。两步：选传输类型并填连接信息，再选信任级别。

### 传输类型

- **本地进程（stdio）**：IcePaw 拉起一个子进程（node / npx / uvx 等），经 stdin/stdout 通信。填启动命令与参数，例如 `npx -y @some/mcp-server`。
- **远程 HTTP（streamable）**：连接一个已经在跑的服务，填它的 URL，例如本机服务 `http://localhost:8000/mcp`。需要鉴权的服务可加自定义请求头（如 `Authorization`）。
- **SSE**：暂未支持，需要时先用 streamable HTTP（多数服务两者都开）。

典型例子——UE 5.8 内置的实验性 MCP server：在 UE 编辑器侧启用插件并开启 server（默认端口 8000），然后在 IcePaw 里添加一个远程 HTTP server，URL 填 `http://localhost:8000/mcp` 即可。编辑器重启后连接失效也没关系（见下「断线自动重连」）。

### 信任级别

- **每次确认（默认）**：该 server 的每个工具调用都弹授权卡。
- **信任**：该 server 的工具免确认直接执行。只对你了解来源的 server 开。

### 工具命名

外部工具在 agent 侧的名字带 `t{序号}_` 前缀（如 `t3_spawn_actor`）——这是多 server 同名工具的消歧机制，调用时正常使用即可。

### 断线自动重连

外部 server 的连接断了（进程退出、服务重启、网络中断），agent 调用工具时会**自动重连一次并重试该调用**（同一 server 30 秒内至多自动重连一次，防止风暴）。streamable HTTP 服务重启后旧会话失效（返回 404）属于此列，会被自动恢复。

仍失败时工具会返回「MCP 连接失败」错误并指路：稍后重试，或到 设置 → MCP / 工具集 手动重试该 server。**手动禁用的 server 不会被自动重连复活**——禁用是明确的管理动作，重新启用需要你操作。

## 授权模型（为什么弹窗）

不是所有工具调用都直接执行。按工具的**授权级别**：

| 级别 | 行为 |
|---|---|
| Always | 直接执行，不问（如纯查询类） |
| PathWhitelist | 访问路径在白名单内才放行，否则弹窗确认 |
| Confirm | 每次都弹窗让你确认（如写文件、跑命令） |

弹窗里你可以**允许 / 拒绝**。允许时可选**生效范围**（会话级记忆，同一会话内不再重复弹，重启后清空）：

- **仅此一次**（默认）：只放行本次。
- **此目录（含子目录）**：该目录下的操作本会话免问。
- **此工具（本会话）**：该工具本会话免问。
- **此 Server（本会话）**：仅外部 server 工具有此档——一次批准后，**该 server 的全部工具**本会话免问。外部 server 常带几十个工具（如 UE），逐个批太碎时用这档。

## workspace 内免授权

agent 自己 **workspace（工作区）内的文件操作免授权**——workspace 是 agent 的信任领地，读写自己目录下的文件不会弹窗。这是为了让 agent 能顺畅地操作自己的项目文件。

> 所以建议给 agent 绑定一个 workspace（项目目录），文件类工具就会在该目录内流畅工作、目录外才确认。

## 常见疑问

- **为什么 read_file 也要确认？** 该文件在 workspace 之外，或工具级别是 Confirm。
- **工具不出现 / 不能用？** 确认会话「启用工具」打开、对应 server 启用、agent 白名单没禁用它。
- **拒绝了怎么办？** agent 会收到「用户拒绝」的结果，通常会换别的方式或停下来问你。
- **工具报「MCP 连接失败」？** server 断线且自动重连未成功。确认服务端还在跑（如 UE 编辑器没关），稍后重试；持续失败到 设置 → MCP / 工具集 手动重试。
- **不想每个工具都点一次？** 外部 server 的授权卡选「此 Server（本会话）」；或在 server 设置里直接把信任级别设为「信任」。

## 相关

- workspace 怎么配、project.md 是什么：见「项目与 workspace」。
- agent.yaml 里怎么限制工具、调轮数上限：见「agent.yaml 进阶配置」。
