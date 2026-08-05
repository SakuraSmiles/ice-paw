---
title: 快速上手（首个 agent 与 API Key）
summary: 从零开始：创建 agent、配置 provider/model/API Key、发送第一条消息。新用户必读。
tags: [快速上手, 新手, 开始, 创建agent, API Key, provider, model, base_url, 第一次使用, 入门]
---

# 快速上手（首个 agent 与 API Key）

ice-paw 通过 **agent** 调用大模型。第一次使用，按这三步就能聊起来。

## 1. 创建 agent 并配置模型

进入 **设置 → Agent**，新建一个 agent，关键配置：

| 字段 | 说明 |
|---|---|
| Provider | 模型服务商（如智谱 GLM、OpenAI、DeepSeek 等，或 Anthropic / OpenAI 兼容端点） |
| Model | 具体模型名，如 `glm-5.2`、`gpt-4o` 等 |
| Base URL | 走兼容端点时填（如智谱 Anthropic 兼容端点）；官方直连一般留空 |
| API Key | 该 provider 的接口密钥，加密保存 |
| System Prompt | agent 的人设/指令（也可在 agent.yaml 里改） |

> API Key 被加密存储，不会明文落盘。

## 2. 回到对话页选择该 agent

新建会话或打开已有会话，确认当前会话绑定的是刚配好的 agent（会话绑 agent，一个 agent 可被多个会话复用）。

## 3. 发送第一条消息

在输入框打字、发送。流式返回；工具调用会自动多轮执行。

## 常见卡点

- **发不出消息 / 报错**：99% 是 API Key 没配、配错，或 provider/model/base_url 不匹配。回设置页核对。
- **用智谱 GLM**：走 Anthropic 兼容端点，provider 选对、填好 base_url 和 Key。
- **响应慢 / 卡住**：见「常见问题」里的「对话卡住怎么办」。

## 进阶

配好基础对话后，可以接着开：
- **知识库（RAG）**：让 agent 检索你的文档 → 见「配置知识库与 embedding」。
- **工具（MCP）**：让 agent 读写文件、跑命令 → 见「配置 MCP 工具」。
- **agent.yaml**：细调人设/温度/预算/钩子 → 见「agent.yaml 进阶配置」。

## 相关

- 想让 agent 查文档回答：见「配置知识库与 embedding」。
- 工作空间是什么：见「项目与 workspace」。
