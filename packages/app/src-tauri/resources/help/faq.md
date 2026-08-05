---
title: 常见问题（FAQ）
summary: 对话卡住/停止、工具一直循环、token 预算、图片支持、日志在哪看、配置文件在哪、数据存哪等高频问题。
tags: [FAQ, 常见问题, 卡住, 停止, token, 预算, 图片, 日志, 循环, 卡死, 配置文件, 数据目录]
---

# 常见问题（FAQ）

## 对话卡住 / 没响应怎么办

- **点「停止」**：触发取消，已生成的内容会保留。
- **工具一直转圈**：可能 agent 在多轮调工具。有保护机制兜底：
  - 工具轮数上限 `tool_max_rounds`（默认 50）——到了自动停。
  - 停滞检测：连续若干轮没进展会自动终止（`finish_reason=stuck`）。
  - Token 预算 `max_total_tokens`（默认 500k）——超了终止。
- 以上三个上限都能在 `agent.yaml` 里调（见「agent.yaml 进阶配置」）。

## 怎么停止生成

对话页的**停止按钮**，或对应会话执行停止。会立即中断流式生成，已输出的文本不丢。

## 能发图片吗

支持。前提是 agent 的模型支持视觉（`supports_vision`），且 provider 支持。限制：图片数量有上限、单张大小约 5MB 内、支持常见格式（png/jpg/jpeg/webp）。不支持的模型/格式会被拦截并提示。

## Token 预算 / 回答被截断

整次对话有个 token 预算上限（默认 `max_total_tokens: 500000`），超了会终止（`finish_reason=budget_exceeded`），避免长对话失控烧 token。多轮工具场景如果提前到顶，可在 agent.yaml 调大这个值。

## 日志在哪看

**设置 → 日志** 页面可以直接看运行日志。日志按天滚动落盘（tracing daily rotate），排查问题（API 报错、工具失败、钩子执行等）很有用。

## 配置文件在哪

- agent 配置：`<agent 工作区>/agent.yaml`（即时生效，不用重启）。
- 知识库文档：`<默认工作空间>/knowledge/`（全局）和 `<agent 工作区>/knowledge/`（agent 专属）。
- 项目上下文：`<默认工作空间>/projects/<项目id>/`（project.md、conventions.md）。

详见「项目与 workspace」。

## 工具调用为什么失败 / 被拒

- **授权被拒**：你在弹窗里点了拒绝，agent 会收到拒绝结果。
- **路径不在 workspace**：需要确认；放进 agent/项目 workspace 内可免授权。
- **工具未启用**：会话没开「启用工具」，或 agent 白名单禁用了它。
- 详见「配置 MCP 工具与授权」。

## 换了模型 / embedding 没生效

- LLM 模型：会话级可临时覆盖；agent 默认模型改 agent 配置。
- embedding 模型：切换会全量重建向量，量大可能几十秒，有二次确认。没配 embedding 则知识库退化为关键词检索。详见「配置知识库与 embedding」。

## 还是搞不定

把「设置 → 日志」里的相关日志带上，描述你做了什么、期望什么、实际什么，便于定位。
