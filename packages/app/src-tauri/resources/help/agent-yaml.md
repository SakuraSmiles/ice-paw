---
title: agent.yaml 进阶配置
summary: agent.yaml 所有字段详解：人设、温度、token 预算、历史窗口、工具轮数上限、工具白名单、hooks 生命周期钩子，以及 Word 文档样式偏好（word_style_profile + 模板目录：workspace templates/ 与软件共享模板目录）。
tags: [agent.yaml, 配置, system_prompt, temperature, max_tokens, hooks, 钩子, 预算, tool_max_rounds, max_total_tokens, word_style_profile, Word, 样式, 模板, 进阶]
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
| `word_style_profile` | string | - | Word 文档样式偏好（见下） |

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

## word_style_profile：Word 文档样式偏好

写 Word 文档（docx）时的个性化约定——**一次定义，处处生效**。非空时每个回合的 system prompt 会自动带上「Word 文档样式偏好」小节，agent 用 edit_docx / insert_table_after 等工具写文档时遵循它。

**两种用法：**

1. **口头提案（推荐）**：直接在对话里告诉 agent 你的偏好，比如「以后正文用宋体小四，表头深蓝底白字，标题黑体」。agent 会通过 `propose_config_change` 发起提案，你批准后偏好自动写进 agent.yaml——之后所有文档都遵循，不用每次重复。

2. **手写 yaml**：直接编辑 `word_style_profile` 多行块：

```yaml
word_style_profile: |
  正文字体：宋体小四（12pt），行距 1.5 倍。
  标题：黑体，一级 16pt / 二级 14pt。
  表格：表头深蓝底白字加粗，隔行浅灰底纹。
```

偏好是自由文字，不解析不校验——写清楚你关心的字体/字号/配色/表格样式即可。摘除 = 删掉这个块（或让 agent 提一次「清除 Word 样式偏好」的提案）。

**配套：templates/ 模板目录**

如果偏好复杂到文字描述不清（整套样式定义、页眉页脚、封面），走模板轨道，模板放两处之一：① workspace 的 `templates/` 目录——项目自用，把模板 docx 放进去即可；② 软件共享模板目录——应用数据目录下的 `templates/`，安装包自带 `formal-report.docx` 正式报告模板（标题四级、表格与列表样式、密级页眉、页码页脚），可直接改它的样式或放自己的模板，全部 agent 共享。agent 建新文档用 `write_docx` 一次调用完成：`template` 参数填模板文件名（如 `formal-report.docx`），依次查 workspace templates/ → 共享目录，都没有再回落内置档位 `report`（同名文件优先于内置）；模板的样式/编号/页面设置原样继承，正文按块序写入并自检后才落盘——这是「继承整套模板」的正路。`word_style_profile` 管的是「每次写内容时的格式纪律」，两条轨道互补。

## 相关

- 工具白名单/授权模型：见「配置 MCP 工具与授权」。
- 对话卡住/轮数/预算：见「常见问题」。
