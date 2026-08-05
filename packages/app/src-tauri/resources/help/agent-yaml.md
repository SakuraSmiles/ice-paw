---
title: agent.yaml 进阶配置
summary: agent.yaml 所有字段详解：人设、温度、token 预算、历史窗口、工具轮数上限、工具白名单，以及 hooks 生命周期钩子（inject_prompt/call_tool/log）。
tags: [agent.yaml, 配置, system_prompt, temperature, max_tokens, hooks, 钩子, 预算, tool_max_rounds, max_total_tokens, 进阶]
---

# agent.yaml 进阶配置

每个 agent 的工作区里有个 `agent.yaml`，改完**即时生效、不用重启**。它覆盖数据库里的 agent 配置（文件优先）。字段都可省略，省略则用默认值或 DB 值。

## 位置

`<agent 工作区>/agent.yaml`。新建 agent 时会自动生成一份默认的。

## 字段速查

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `system_prompt` | string | agent 人设 | 系统提示词，定义 agent 角色/行为 |
| `description` | string | - | agent 描述（展示用） |
| `temperature` | number | 继承 | 采样温度，越高越发散 |
| `max_tokens` | int | 继承 | 单次回复最大 token |
| `max_history_messages` | int | 系统默认 | 注入上下文的历史消息条数上限 |
| `tool_max_rounds` | int | 50 | 工具调用最大轮数（防止无限循环） |
| `max_total_tokens` | int | 500000 | 整次对话 token 预算上限，超了终止 |
| `enabled_tools` | string[] | 全部启用 | 工具白名单；空数组=禁用所有工具 |
| `cache_prompt` | bool | 继承 | 开启 prompt caching（Anthropic 显式断点） |
| `supports_vision` | bool | 继承 | 是否支持图片输入 |
| `hooks` | map | - | 生命周期钩子（见下） |

## 示例

```yaml
system_prompt: |
  你是一个严谨的代码助手，回答前先复述问题。
temperature: 0.3
max_tokens: 4096
tool_max_rounds: 30
max_total_tokens: 800000
enabled_tools:
  - read_file
  - search_kb
```

## hooks：生命周期钩子

在对话的 4 个时机自动执行内置动作，用来**稳定 agent 行为**（比如每轮强制格式、记录日志、触发工具）。配置在 `hooks` 下，键是时机，值是动作列表。

**4 个时机（hook point）：**
- `conversation_start`：对话开始（上下文拼装后）
- `before_llm`：每轮请求 LLM 前（**核心**——每轮注入规范，防止跑偏）
- `after_tool`：每次工具执行后
- `conversation_end`：对话结束（成功/取消/出错都触发）

**3 种动作：**
- `inject_prompt`：注入一段 prompt（`conversation_start` 追加进 system；`before_llm` 每轮临时注入）
- `log`：记一条日志
- `call_tool`：自动调用某工具（注意：绕过授权弹窗，挑免授权的工具用）

**示例：每轮强制 JSON 输出 + 结束记日志**

```yaml
hooks:
  before_llm:
    - action: inject_prompt
      content: "必须以合法 JSON 输出，不要多余解释。"
  conversation_end:
    - action: log
      message: "一次对话结束"
```

> 钩子是 fail-safe 的：任何动作失败只记警告，**不会中断对话**。没配 `hooks` 则完全无开销。

## 相关

- 工具白名单/授权模型：见「配置 MCP 工具与授权」。
- 对话卡住/轮数/预算：见「常见问题」。
