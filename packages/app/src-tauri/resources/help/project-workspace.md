---
title: 项目与 workspace（工作区）
summary: workspace 工作区是什么、默认工作空间、agent 工作区、项目 workspace、project.md/conventions.md 项目上下文、knowledge 目录约定。
tags: [workspace, 工作区, 工作空间, 项目, project, project.md, conventions, 目录, 默认工作空间, 知识库目录]
---

# 项目与 workspace（工作区）

「workspace（工作区）」是 ice-paw 组织文件、知识库、项目上下文的根目录概念。理解它能解释「agent 为什么读这个目录」「知识库文档放哪」。

## 三层目录

1. **默认工作空间（default workspace）**：在设置里设的全局根目录。全局知识库挂在它的 `knowledge/` 下；agent 没单独设工作区时也回退到这里。
2. **agent 工作区**：单个 agent 的工作目录。agent.yaml、agent 专属知识库都在这里。没设则回退到 `<默认工作空间>/agents/<agent_id>/`。
3. **项目 workspace**：项目绑定的源码根目录。绑了之后，文件/代码类工具（read_file、run_command、git 等）把它当 current_dir 和路径白名单根。

## knowledge 目录约定

知识库**按约定自动建库**，不用手动创建：

- 全局知识库：`<默认工作空间>/knowledge/`（所有 agent 共享）
- agent 知识库：`<agent 工作区>/knowledge/`（仅该 agent）

把 `.md` 丢进去就自动索引。详见「配置知识库与 embedding」。

## 项目上下文（project.md / conventions.md）

每个项目在 `<默认工作空间>/projects/<项目id>/` 下有 IcePaw 管理的上下文目录（**不在你的项目源码目录里**，避免污染/误删你的代码）：

- `project.md`：项目说明（技术栈、架构、业务背景）。新建项目时自动生成模板，自己填。
- `conventions.md`：编码规范（命名、格式、最佳实践）。

这两个文件的内容会自动注入 agent 的系统上下文，让 agent 了解你的项目。**改完即时生效**。

## workspace 内免授权

agent 在**自己 workspace 内**的文件操作免授权（不弹窗）；workspace 外才需要确认。所以给 agent / 项目绑定 workspace，文件工具会更流畅。详见「配置 MCP 工具与授权」。

## 相关

- 知识库怎么用、embedding 怎么配：见「配置知识库与 embedding」。
- 工具授权模型：见「配置 MCP 工具与授权」。
