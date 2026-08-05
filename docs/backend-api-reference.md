# IcePaw Backend API Reference

## 概述

本文档梳理了 IcePaw 后端（`packages/app/src-tauri/src/`）所有 `#[tauri::command]` 标注的 Tauri Commands API。

### 统计
- **已注册命令总数**：34 个
- **未注册但已定义**：3 个（`list_providers` / `list_models` / `list_tool_defs`）
- **仅后端内部使用**：1 个（`count_messages`，`#[allow(dead_code)]`）
- **后端向前端 emit 的事件**：14 个（含 `chat:config-proposal` / `chat:config-proposal-response`）

### 命令注册位置
所有已注册命令在 `lib.rs` 的 `tauri::generate_handler![...]` 中集中管理。

### 模块分布

| 模块 | 文件 | 命令数（已注册） | 说明 |
|------|------|-----------------|------|
| Agent | `agent_cmd.rs` | 5 | Agent 的 CRUD + API Key 轮换 |
| Chat | `chat_cmd.rs` | 2 | 发送消息 + 停止生成 |
| Conversation | `conversation_cmd.rs` | 6 | 会话的 CRUD + 工具覆盖 |
| Message | `message_cmd.rs` | 2 | 消息列表 + 创建 |
| Preferences | `preferences_cmd.rs` | 2 | 全局偏好设置 |
| Project | `project_cmd.rs` | 12 | 项目空间管理 |
| Template | `template_cmd.rs` | 5 | Prompt 模板 CRUD |
| Provider | `provider_cmd.rs` | 2（未注册） | 厂商/模型元信息 |
| Tool | `tool_cmd.rs` | 1（未注册） | 工具定义列表 |
| MCP | `mcp_cmd.rs` | 7 | MCP Server 配置 CRUD + 重启 + 工具列表 |

---

## 模块一：Agent（Agent 管理）

### list_agents

- **参数**：无（从 `State<Arc<dyn AgentCmd>>` 注入）
- **返回**：`AppResult<Vec<Agent>>`
- **说明**：列出全部 Agent（已屏蔽 API Key 引用）。Agent 结构体含 name/provider/model/temperature/max_tokens/enabled_tools/cache_prompt/supports_vision/has_api_key 等字段。
- **前端使用状态**：✅ 已使用（`bridge.agents.list()`）

### create_agent

- **参数**：
  - `input: NewAgent` — `{ name, provider, model, api_key, system_prompt?, base_url?, temperature?, max_tokens?, extra_params?, sort_order?, cache_prompt?, max_history_messages?, tool_trim_threshold?, enabled_tools?, supports_vision? }`
- **返回**：`AppResult<Agent>`
- **说明**：创建新 Agent，api_key 加密存入 stronghold。
- **前端使用状态**：✅ 已使用（`bridge.agents.create(input)`）

### update_agent

- **参数**：
  - `input: AgentUpdate` — `{ id, name?, provider?, model?, system_prompt?, base_url? (双层Option), temperature?, max_tokens?, extra_params?, sort_order?, cache_prompt?, max_history_messages? (双层Option), tool_trim_threshold? (双层Option), enabled_tools? (双层Option), supports_vision? }`
- **返回**：`AppResult<Agent>`
- **说明**：部分更新 Agent 配置。使用双层 Option 语义：外层 None = 不更新该字段，外层 Some(内层 None) = 清空该字段。
- **前端使用状态**：✅ 已使用（`bridge.agents.update(input)`）

### rotate_agent_api_key

- **参数**：
  - `input: RotateAgentKey` — `{ agent_id, api_key, base_url? }`
- **返回**：`AppResult<Agent>`
- **说明**：轮换 Agent 的 API Key（更新 stronghold 中存储的值）。
- **前端使用状态**：✅ 已使用（`bridge.agents.rotateKey(input)`）

### delete_agent

- **参数**：
  - `id: String`
- **返回**：`AppResult<()>`
- **说明**：删除指定 Agent（stronghold 中的 key + DB 行，前端调用前会确认）。
- **前端使用状态**：✅ 已使用（`bridge.agents.delete(id)`）

---

## 模块二：Chat（聊天编排）

### send_message

- **参数**：
  - `input: SendMessageInput` — `{ conversation_id, content?, content_blocks?, tools_enabled?, model? }`
  - 注：`content_blocks` 优先于 `content`，至少提供其一；`model` 为会话级 model 覆盖（None=使用 Agent 默认）
- **返回**：`AppResult<()>`（立即返回，流式结果通过事件下发）
- **说明**：核心聊天入口。流程：入参校验 → 取 Agent+API Key → 拼装 Pipeline 上下文 → 写用户消息+助手占位 → emit `chat:start` → spawn 流式协程。
- **前端使用状态**：✅ 已使用（`bridge.chat.sendMessage(input)`）

#### 关联事件（后端 → 前端，非 Tauri Command，但前端必须订阅）

| 事件名 | Payload 类型 | 说明 |
|--------|------------|------|
| `chat:start` | `ChatStartPayload { conversation_id, user_message_id, assistant_message_id }` | 流式开始 |
| `chat:chunk` | `ChatChunkPayload { conversation_id, message_id, delta }` | 文本增量 |
| `chat:done` | `ChatDonePayload { conversation_id, message_id, finish_reason, usage? }` | 流结束 |
| `chat:error` | `ChatErrorPayload { conversation_id, message_id, kind, message }` | 错误 |
| `chat:retrying` | `ChatRetryingPayload { conversation_id, message_id, attempt, max_attempts, reason }` | 重试中 |
| `chat:round-state` | `ChatRoundStatePayload { conversation_id, round, elapsed_ms, tokens_prompt, tokens_completion, cached_tokens, retry_count }` | 轮次统计 |
| `chat:thinking` | `ChatThinkingPayload { conversation_id, message_id, content }` | 思考过程增量 |
| `chat:tool-call-start` | `ChatToolCallStartPayload { conversation_id, message_id, id, name }` | 工具调用开始 |
| `chat:tool-call-delta` | `ChatToolCallDeltaPayload { conversation_id, message_id, id, delta }` | 工具参数增量 |
| `chat:tool-call-end` | `ChatToolCallEndPayload { conversation_id, message_id, id }` | 工具参数完毕 |
| `chat:tool-result` | `ChatToolResultPayload { conversation_id, message_id, tool_use_id, content, is_error }` | 工具执行结果 |
| `chat:summary-injected` | `ChatSummaryInjectedPayload { conversation_id, summary_tokens, original_count, kept_count }` | 摘要注入完成 |
| `chat:tool-auth-request` | `ToolAuthRequestPayload` | 工具执行需授权（Rust → 前端） |
| （前端 emit）`chat:tool-auth-response` | `ToolAuthResponse { request_id, allowed }` | 工具授权响应（前端 → Rust） |
| `chat:config-proposal` | `ConfigProposalPayload { request_id, conversation_id, message_id, tool_use_id, sensitivity, action, summary }` | 配置提案请求（Rust → 前端） |
| （前端 emit）`chat:config-proposal-response` | `ConfigProposalResponse { request_id, decision }` | 配置提案响应（前端 → Rust） |

### stop_generation

- **参数**：
  - `conversation_id: String`
- **返回**：`AppResult<()>`
- **说明**：触发指定会话的 CancellationToken，中断正在进行的流式生成。已停止的会话调用无副作用。
- **前端使用状态**：✅ 已使用（`bridge.chat.stopGeneration(conversationId)`）

---

## 模块三：Conversation（会话管理）

### list_conversations

- **参数**：
  - `agent_id: String`
- **返回**：`AppResult<Vec<Conversation>>`
- **说明**：列出指定 Agent 下的全部会话（pinned desc, updated_at desc）。
- **前端使用状态**：✅ 已使用（`bridge.conversations.list(agentId)`）

### create_conversation

- **参数**：
  - `input: NewConversation` — `{ agent_id, title?, project_id? }`
- **返回**：`AppResult<Conversation>`
- **说明**：创建新会话（自动生成 UUID id）。
- **前端使用状态**：✅ 已使用（`bridge.conversations.create(input)`）

### rename_conversation

- **参数**：
  - `id: String`
  - `title: String`
- **返回**：`AppResult<()>`
- **说明**：重命名会话。
- **前端使用状态**：✅ 已使用（`bridge.conversations.rename(id, title)`）

### pin_conversation

- **参数**：
  - `id: String`
  - `pinned: bool`
- **返回**：`AppResult<()>`
- **说明**：置顶 / 取消置顶会话。
- **前端使用状态**：✅ 已使用（`bridge.conversations.pin(id, pinned)`）

### delete_conversation

- **参数**：
  - `id: String`
- **返回**：`AppResult<()>`
- **说明**：删除会话（级联清理 messages / tool_calls）。
- **前端使用状态**：✅ 已使用（`bridge.conversations.delete(id)`）

### update_conversation_tools_override

- **参数**：
  - `conversation_id: String`
  - `tools_override: Option<HashMap<String, bool>>`
- **返回**：`AppResult<()>`
- **说明**：更新对话级工具覆盖配置。None = 清除覆盖，恢复继承 Agent 配置；Some(map) = 写入 per-tool 勾选状态。
- **前端使用状态**：✅ 已使用（`bridge.conversations.updateToolsOverride(conversationId, toolsOverride)`）

---

## 模块四：Message（消息管理）

### list_messages

- **参数**：
  - `conversation_id: String`
  - `limit: Option<i64>`（上限 1000，默认 100）
  - `before: Option<serde_json::Value>`（复合游标 `[created_at, rowid]`，前端传 JS 数组）
- **返回**：`AppResult<Vec<Message>>`
- **说明**：列出会话内消息，支持复合游标分页。`before` 格式为 `[created_at_str, rowid_int]`，由前端从上一页最末一条消息的对应字段回传。
- **前端使用状态**：✅ 已使用（`bridge.messages.list(conversationId, opts)`）

### create_message

- **参数**：
  - `input: NewMessage` — `{ conversation_id, role, content, token_count?, error?, model? }`
- **返回**：`AppResult<Message>`
- **说明**：手动写入一条消息（自动生成 UUID id）。需校验 conversation_id 非空 + content 非空。
- **前端使用状态**：✅ 已使用（`bridge.messages.create(input)`）

---

## 模块五：Preferences（全局偏好）

### get_preferences

- **参数**：无（从 `State<SqlitePool>` 注入）
- **返回**：`AppResult<UserPreferences>`
- **说明**：读取全部用户偏好设置。UserPreferences 含：default_agent_id, default_template_id, on_startup, language, theme, code_theme, font_size。
- **前端使用状态**：✅ 已使用（`bridge.preferences.get()`）

### set_preference

- **参数**：
  - `key: String`
  - `value: String`（JSON.stringify 后的字符串）
- **返回**：`AppResult<()>`
- **说明**：更新单个偏好项。value 需前端 JSON.stringify 后传入。
- **前端使用状态**：✅ 已使用（`bridge.preferences.set(key, value)`）

---

## 模块六：Project（项目空间管理）

### list_projects

- **参数**：无（从 `State<SqlitePool>` 注入）
- **返回**：`AppResult<Vec<Project>>`
- **说明**：列出全部项目（含每个项目下的 Agent 成员列表）。
- **前端使用状态**：✅ 已使用（`bridge.projects.list()`）

### create_project

- **参数**：
  - `input: NewProject` — `{ name, description?, icon?, workspace_path? }`（不含 members）
- **返回**：`AppResult<Project>`
- **说明**：创建项目（仅基础信息，不含 members）。保留作向后兼容，新弹窗主流程走 `create_project_with_agents`。
- **前端使用状态**：✅ 已使用（`bridge.projects.create(input)`）

### create_project_with_agents

- **参数**：
  - `input: NewProject` — `{ name, description?, icon?, workspace_path?, agents: [{ agent_id, role }] }`
- **返回**：`AppResult<Project>`（含 agents 字段）
- **说明**：创建项目 + 一次性写入初始 Agent 成员（事务保证原子性）。role 仅支持 "lead"/"member"。推荐入口。
- **前端使用状态**：✅ 已使用（`bridge.projects.createWithAgents(input)`）

### update_project

- **参数**：
  - `id: String`
  - `name: Option<String>`
  - `description: Option<String>`
- **返回**：`AppResult<Project>`
- **说明**：简单更新项目（仅 name/description）。保留旧 command，新流程走 `update_project_full`。
- **前端使用状态**：✅ 已使用（`bridge.projects.update(id, name, description)`）

### update_project_full

- **参数**：
  - `id: String`
  - `patch: ProjectPatch` — `{ name?, description? (双层Option), icon? (双层Option), workspace_path? (双层Option) }`
  - `members: Option<Vec<ProjectAgentInput>>` — None=不动，Some([])=清空，Some([{ agent_id, role }])=替换
- **返回**：`AppResult<Project>`（含 agents 字段）
- **说明**：原子更新项目（字段 + 可选成员替换），事务保证全成功或全回滚。推荐入口。
- **前端使用状态**：✅ 已使用（`bridge.projects.updateFull(id, patch, members)`）

### set_project_agents

- **参数**：
  - `project_id: String`
  - `agents: Vec<ProjectAgentInput>` — `[{ agent_id, role }]`
- **返回**：`AppResult<()>`
- **说明**：整体替换项目的 Agent 成员。传空数组 = 清空全部成员。事务保证原子性。会做去重校验。
- **前端使用状态**：✅ 已使用（`bridge.projects.setAgents(projectId, agents)`）

### delete_project

- **参数**：
  - `id: String`
- **返回**：`AppResult<()>`
- **说明**：删除项目（project_agents CASCADE 删除，conversations.project_id → NULL）。
- **前端使用状态**：✅ 已使用（`bridge.projects.delete(id)`）

### reorder_projects

- **参数**：
  - `ordered_ids: Vec<String>`
- **返回**：`AppResult<()>`
- **说明**：批量更新项目的 sort_order（按传入的 id 顺序）。
- **前端使用状态**：✅ 已使用（`bridge.projects.reorder(orderedIds)`）

### add_project_agent

- **参数**：
  - `project_id: String`
  - `agent_id: String`
  - `role: Option<String>`（默认 "member"）
- **返回**：`AppResult<()>`
- **说明**：细粒度入口，添加单个 Agent 到项目。弹窗主流程不走此接口。
- **前端使用状态**：✅ 已使用（`bridge.projects.addAgent(projectId, agentId, role)`）

### remove_project_agent

- **参数**：
  - `project_id: String`
  - `agent_id: String`
- **返回**：`AppResult<()>`
- **说明**：细粒度入口，从项目移除单个 Agent。
- **前端使用状态**：✅ 已使用（`bridge.projects.removeAgent(projectId, agentId)`）

### list_conversations_by_project

- **参数**：
  - `project_id: Option<String>`（None = 默认项目）
- **返回**：`AppResult<Vec<Conversation>>`
- **说明**：列出某项目下的全部会话。
- **前端使用状态**：✅ 已使用（`bridge.projects.listConversations(projectId)`）

### move_conversation_to_project

- **参数**：
  - `conversation_id: String`
  - `project_id: Option<String>`（None = 移回默认项目）
- **返回**：`AppResult<()>`
- **说明**：移动会话到指定项目。
- **前端使用状态**：✅ 已使用（`bridge.projects.moveConversation(conversationId, projectId)`）

---

## 模块七：Template（Prompt 模板）

### list_templates

- **参数**：无（从 `State<SqlitePool>` 注入）
- **返回**：`AppResult<Vec<Template>>`
- **说明**：列出全部模板。Template 含 id/name/description/system_prompt/user_prompt_prefix/variables/tools。
- **前端使用状态**：✅ 已使用（`bridge.templates.list()`）

### get_template

- **参数**：
  - `id: String`
- **返回**：`AppResult<Template>`
- **说明**：按 ID 取一条模板。
- **前端使用状态**：✅ 已使用（`bridge.templates.get(id)`）

### create_template

- **参数**：
  - `input: NewTemplate` — `{ name, description?, system_prompt?, user_prompt_prefix?, variables?, tools?, sort_order? }`
- **返回**：`AppResult<Template>`
- **说明**：创建模板。name 非空校验。
- **前端使用状态**：✅ 已使用（`bridge.templates.create(input)`）

### update_template

- **参数**：
  - `input: TemplateUpdate` — `{ id, name?, description?, system_prompt?, user_prompt_prefix?, variables?, tools?, sort_order? }`
- **返回**：`AppResult<Template>`
- **说明**：部分更新模板。字段传 None = 不更新该字段；传 Some(非空) = 覆盖。
- **前端使用状态**：✅ 已使用（`bridge.templates.update(input)`）

### delete_template

- **参数**：
  - `id: String`
- **返回**：`AppResult<()>`
- **说明**：删除模板。
- **前端使用状态**：✅ 已使用（`bridge.templates.delete(id)`）

---

## 模块八：Provider（厂商元信息）⚠️ 未注册

### list_providers

- **参数**：无
- **返回**：`AppResult<Vec<ProviderInfo>>`
- **说明**：列出全部可用 Provider（id + 展示名）。编译期常量，不涉及网络/数据库。
- **前端使用状态**：❌ 未接入（前端使用硬编码的 PROVIDERS 常量数组，未调用此 API；且该命令**未在 lib.rs 注册**）

### list_models

- **参数**：
  - `provider_id: String`
- **返回**：`AppResult<Vec<ModelInfo>>`
- **说明**：列出某 Provider 的可选模型列表。未知 provider 返回空数组。
- **前端使用状态**：❌ 未接入（前端使用硬编码的 MODEL_PRESETS 映射，未调用此 API；且该命令**未在 lib.rs 注册**）

---

## 模块九：Tool（工具定义）⚠️ 未注册

### list_tool_defs

- **参数**：无
- **返回**：`AppResult<Vec<ToolDefWithDanger>>`
- **说明**：列出所有已注册工具的定义（含 danger_level 字段）。内置工具：read_file、list_directory。
- **前端使用状态**：❌ 未接入（且该命令**未在 lib.rs 注册**）

---

## 模块十：MCP（MCP Server 配置管理）

### list_mcp_servers

- **参数**：无（从 `State<SqlitePool>` 注入）
- **返回**：`AppResult<Vec<McpServerConfig>>`
- **说明**：列出全部已配置的 MCP Server。McpServerConfig 含 id/name/description/command/args/env/enabled/trust_level/created_at/updated_at。
- **前端使用状态**：✅ 已使用（`bridge.mcp.list()`）

### create_mcp_server

- **参数**：`input: NewMcpServer` — `{ id, name, description?, command, args?, env?, enabled?, trust_level? }`
- **返回**：`AppResult<McpServerConfig>`
- **说明**：新增 MCP Server 配置并启动 stdio 连接（trust_level: "trusted" | "untrusted"）。
- **前端使用状态**：✅ 已使用（`bridge.mcp.create(input)`）

### update_mcp_server

- **参数**：`input: McpServerUpdate` — `{ id, name?, description?, command?, args?, env?, enabled?, trust_level? }`
- **返回**：`AppResult<McpServerConfig>`
- **说明**：部分更新配置；若 command/args 变更会重启连接。
- **前端使用状态**：✅ 已使用（`bridge.mcp.update(input)`）

### delete_mcp_server

- **参数**：`id: String`
- **返回**：`AppResult<()>`
- **说明**：删除 MCP Server 配置并停止连接。
- **前端使用状态**：✅ 已使用（`bridge.mcp.remove(id)`）

### restart_mcp_server

- **参数**：`id: String`
- **返回**：`AppResult<()>`
- **说明**：重启指定 MCP Server 的 stdio 连接（重新 spawn 子进程 + initialize 握手）。
- **前端使用状态**：✅ 已使用（`bridge.mcp.restart(id)`）

### list_active_mcp_servers

- **参数**：无（从 `State<Arc<McpServerManager>>` 注入）
- **返回**：`AppResult<Vec<(String, String)>>` — `(server_id, name)` 列表
- **说明**：列出当前活跃（已连接）的 MCP Server。
- **前端使用状态**：✅ 已使用（`bridge.mcp.listActive()`）

### list_mcp_server_tools

- **参数**：`id: String`
- **返回**：`AppResult<Vec<McpToolDef>>` — 每项 `{ name, description, input_schema }`
- **说明**：列出指定 MCP Server 提供的工具定义（来自 tools/list，manager 缓存 tools_cache）。
- **前端使用状态**：✅ 已使用（`bridge.mcp.listTools(id)`）

---

## 附录：错误类型（AppError）

所有命令返回 `Result<T, AppError>`，AppError 枚举如下：

| 变体 | 说明 | 前端识别方式 |
|------|------|------------|
| `Database(String)` | 数据库操作失败 | kind="database" |
| `Stronghold(String)` | Stronghold vault 读写错误 | kind="stronghold" |
| `NotFound { resource, id }` | 资源不存在 | kind="not_found" |
| `Validation(String)` | 入参校验失败（业务级，前端可读） | kind="validation" |
| `Json(String)` | 序列化/反序列化错误 | kind="json" |
| `Io(String)` | 文件 IO 错误 | kind="io" |
| `Tauri(String)` | 框架错误 | kind="tauri" |
| `Internal(String)` | 兜底：未分类内部错误 | kind="internal" |

前端统一通过 `bridge.ts` 中的 `wrapInvokeError(op, err)` 处理，提取 `err.message` 用于 toast 提示。

---

## 附录：前端 Bridge 调用速查

前端所有 invoke 调用统一通过 `packages/app/src/api/bridge.ts` 的 `bridge` 对象，命名空间映射：

| JS 命名空间 | Rust 命令名 |
|------------|------------|
| `bridge.agents.list()` | `list_agents` |
| `bridge.agents.create(input)` | `create_agent` |
| `bridge.agents.update(input)` | `update_agent` |
| `bridge.agents.rotateKey(input)` | `rotate_agent_api_key` |
| `bridge.agents.delete(id)` | `delete_agent` |
| `bridge.conversations.list(agentId)` | `list_conversations` |
| `bridge.conversations.create(input)` | `create_conversation` |
| `bridge.conversations.rename(id, title)` | `rename_conversation` |
| `bridge.conversations.pin(id, pinned)` | `pin_conversation` |
| `bridge.conversations.delete(id)` | `delete_conversation` |
| `bridge.conversations.updateToolsOverride(...)` | `update_conversation_tools_override` |
| `bridge.messages.list(conversationId, opts)` | `list_messages` |
| `bridge.messages.create(input)` | `create_message` |
| `bridge.chat.sendMessage(params)` | `send_message` |
| `bridge.chat.stopGeneration(conversationId)` | `stop_generation` |
| `bridge.templates.list()` | `list_templates` |
| `bridge.templates.get(id)` | `get_template` |
| `bridge.templates.create(input)` | `create_template` |
| `bridge.templates.update(input)` | `update_template` |
| `bridge.templates.delete(id)` | `delete_template` |
| `bridge.preferences.get()` | `get_preferences` |
| `bridge.preferences.set(key, value)` | `set_preference` |
| `bridge.projects.list()` | `list_projects` |
| `bridge.projects.create(input)` | `create_project` |
| `bridge.projects.createWithAgents(input)` | `create_project_with_agents` |
| `bridge.projects.update(id, name, desc)` | `update_project` |
| `bridge.projects.updateFull(id, patch, members)` | `update_project_full` |
| `bridge.projects.setAgents(projectId, agents)` | `set_project_agents` |
| `bridge.projects.delete(id)` | `delete_project` |
| `bridge.projects.reorder(orderedIds)` | `reorder_projects` |
| `bridge.projects.addAgent(...)` | `add_project_agent` |
| `bridge.projects.removeAgent(...)` | `remove_project_agent` |
| `bridge.projects.listConversations(projectId)` | `list_conversations_by_project` |
| `bridge.projects.moveConversation(convId, projectId)` | `move_conversation_to_project` |
| `bridge.mcp.list()` | `list_mcp_servers` |
| `bridge.mcp.create(input)` | `create_mcp_server` |
| `bridge.mcp.update(input)` | `update_mcp_server` |
| `bridge.mcp.remove(id)` | `delete_mcp_server` |
| `bridge.mcp.restart(id)` | `restart_mcp_server` |
| `bridge.mcp.listActive()` | `list_active_mcp_servers` |
| `bridge.mcp.listTools(id)` | `list_mcp_server_tools` |

---

## 附录：核心数据类型速查

### ContentBlock（消息内容块）

```rust
#[serde(tag = "type")]
enum ContentBlock {
    Text { text: String },
    Image { data: String, media_type: String },  // data 为裸 base64，不含前缀
    ToolUse { id: String, name: String, input: String },
    ToolResult { tool_use_id: String, content: String, is_error?: bool },
    Thinking { thinking: String, signature?: String },
}
```

### SendMessageInput

```rust
struct SendMessageInput {
    conversation_id: String,
    content?: String,           // 旧接口：纯文本
    content_blocks?: Vec<ContentBlock>,  // 新接口：多模态块（优先）
    tools_enabled?: bool,       // 是否启用工具调用
    model?: String,             // 会话级模型覆盖（None=用Agent默认）
}
```

### Agent

```rust
struct Agent {
    id: String, name: String, provider: String, model: String,
    system_prompt: String, base_url?: String, temperature: f64,
    max_tokens: i32, extra_params: JsonValue, sort_order: i32,
    cache_prompt: bool, max_history_messages?: i32,
    tool_trim_threshold?: i32, enabled_tools?: Vec<String>,
    supports_vision: bool, embedding_model?: String,
    description: String, avatar?: String,
    has_api_key: bool, created_at: String, updated_at: String,
}
```

### Conversation

```rust
struct Conversation {
    id: String, agent_id: String, title: String, pinned: bool,
    tools_override?: HashMap<String, bool>,
    project_id?: String,
    created_at: String, updated_at: String,
}
```

### Message

```rust
struct Message {
    id: String, conversation_id: String, role: String,
    content: String, content_blocks: String,  // content_blocks 为 JSON 字符串
    token_count?: i32, error?: String,
    rowid: i64,  // 分页游标用（内部字段）
    summary_id?: String, model?: String,
    created_at: String,
}
```

### Project

```rust
struct Project {
    id: String, name: String, description: String, icon: String,
    workspace_path?: String, sort_order: i32,
    agents: Vec<ProjectMember>,
    created_at: String, updated_at: String,
}
struct ProjectMember { agent_id: String, role: String }
```

### Template

```rust
struct Template {
    id: String, name: String, description: String,
    system_prompt: String, user_prompt_prefix: String,
    variables: Vec<TemplateVariable>, tools: Vec<String>,
    sort_order: i32, created_at: String, updated_at: String,
}
struct TemplateVariable {
    name: String, label: String, type: String,  // text|textarea|select
    default?: String, options?: Vec<String>,
}
```

### UserPreferences

```rust
struct UserPreferences {
    default_agent_id?: String, default_template_id?: String,
    on_startup?: String, language?: String,
    theme?: String, code_theme?: String, font_size?: i32,
}
```

### ProviderInfo / ModelInfo（未注册 API 的返回类型）

```rust
struct ProviderInfo { id: String, name: String }
struct ModelInfo { id: String, name: String }
```

### ToolDefWithDanger（未注册 API 的返回类型）

```rust
struct ToolDefWithDanger {
    name: String, description: String,
    parameters: JsonValue,  // JSON Schema
    danger_level: String,   // "safe" | "caution" | "dangerous"
}
```
