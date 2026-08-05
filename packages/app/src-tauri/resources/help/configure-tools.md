---
title: 配置 MCP 工具与授权
summary: 怎么启用工具、MCP server 是什么、global/per_agent scope 区别、为什么工具调用会弹授权窗、workspace 内免授权。
tags: [工具, MCP, MCP server, 授权, 权限, 确认弹窗, scope, per_agent, global, 文件工具, run_command]
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

## 授权模型（为什么弹窗）

不是所有工具调用都直接执行。按工具的**授权级别**：

| 级别 | 行为 |
|---|---|
| Always | 直接执行，不问（如纯查询类） |
| PathWhitelist | 访问路径在白名单内才放行，否则弹窗确认 |
| Confirm | 每次都弹窗让你确认（如写文件、跑命令） |

弹窗里你可以**允许 / 拒绝**。允许后，**同一会话内**同一路径不再重复弹（会话级记忆）。

## workspace 内免授权

agent 自己 **workspace（工作区）内的文件操作免授权**——workspace 是 agent 的信任领地，读写自己目录下的文件不会弹窗。这是为了让 agent 能顺畅地操作自己的项目文件。

> 所以建议给 agent 绑定一个 workspace（项目目录），文件类工具就会在该目录内流畅工作、目录外才确认。

## 常见疑问

- **为什么 read_file 也要确认？** 该文件在 workspace 之外，或工具级别是 Confirm。
- **工具不出现 / 不能用？** 确认会话「启用工具」打开、对应 server 启用、agent 白名单没禁用它。
- **拒绝了怎么办？** agent 会收到「用户拒绝」的结果，通常会换别的方式或停下来问你。

## 相关

- workspace 怎么配、project.md 是什么：见「项目与 workspace」。
- agent.yaml 里怎么限制工具、调轮数上限：见「agent.yaml 进阶配置」。
