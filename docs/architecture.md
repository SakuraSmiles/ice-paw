# IcePaw 系统架构

## 分层架构

```
┌─────────────────────────────────────────────┐
│  UI Layer (Vue 3 + Pinia + Vue Router)      │
│  components/  pages/  stores/  composables/ │
├─────────────────────────────────────────────┤
│  Bridge Layer (api/bridge.ts)               │
│  Tauri IPC (invoke + event listen/emit)     │
├─────────────────────────────────────────────┤
│  Command Layer (commands/*.rs)              │
│  Tauri #[command] — 薄编排层，不含业务逻辑    │
├─────────────────────────────────────────────┤
│  Harness Layer (harness/*.rs)               │
│  LLM Provider / Loop Engine / MCP / KB      │
├─────────────────────────────────────────────┤
│  Context Pipeline (context/*.rs)            │
│  Template → OS → SystemPrompt → History     │
│  → Memory → FinalAssemble                   │
├─────────────────────────────────────────────┤
│  Data Layer (db/*.rs)                       │
│  SQLite via sqlx — models / repo / migrate  │
└─────────────────────────────────────────────┘
```

## 核心数据流

### 用户发送消息

```
ChatInput.vue
  → chatStore.sendMessage()
    → bridge.chat.sendMessage()
      → Tauri invoke("send_message", input)
        → chat_cmd::send_message()
          1. 入参校验（content_blocks vs legacy content）
          2. 取会话 → 取 agent → 取 api_key → 创建 LLM provider
          3. 注册 CancellationToken（scopeguard RAII 守卫）
          4. PipelineRunner::default_pipeline().run()
             ├─ TemplateStage: 加载 agent template
             ├─ OsContextStage: OS/时区/工作区信息
             ├─ SystemPromptStage: 四级优先 system prompt
             ├─ HistoryStage: 加载 + 截断历史消息
             ├─ MemoryStage: 滚动摘要（超 token 阈值）
             └─ FinalAssembleStage: 拼装最终 messages
          5. 写 user 消息 + assistant 占位到 DB
          6. emit("chat:start") → 前端开始显示占位
          7. spawn stream_loop 协程
             ├─ stream_loop() wrapper: 创建 BatchWriter
             └─ stream_loop_inner(): 主循环
                 ├─ 'retry_loop: LLM stream_chat + consume_stream
                 ├─ 工具执行: execute_tool_round
                 ├─ 停滞检测: compute_round_key + should_terminate_stuck
                 └─ finalize_success/finalize_error/finalize_cancel
```

### 流式响应推送到前端

```
stream_loop_inner
  → provider.stream_chat()
    → parse_sse_stream()  [anthropic/streaming.rs 或 openai/streaming.rs]
      → tx.send(ChatDelta::Delta { content })
        → stream_consumer
          → app.emit("chat:chunk", payload)
            → chat.ts: listen("chat:chunk")
              → streamingText += delta
                → ChatMessages.vue 响应式渲染
```

## 关键设计决策

### Project 是可选容器

`conversations.project_id` 可 NULL（= "散落会话"）。不强制所有会话属于项目，保持灵活性。删除项目时 `ON DELETE SET NULL` 使会话回归散落而非丢失。

### 双层 workspace 按工具家族分工

- `project.workspace_path` — 文件工具（read_file/write_file/list_directory）
- `agent.workspace_path` — 知识库 + agent 配置工具（search_kb/read_agent_config）
- 工具通过 `ToolContext` 获取 workspace，`ToolContext.workspace` 优先 project 回退 agent

### MCP 工具系统

三层授权模型：
- `Always` — 安全只读操作（git status、web fetch）
- `PathWhitelist` — 限定目录的文件操作（预留）
- `Confirm` — 危险操作需用户逐条批准（shell 命令）

外部 MCP Server 通过 stdio JSON-RPC 连接。子进程环境经白名单过滤（`build_safe_env`），防止 API key 泄漏。

### 项目归档而非删除

`projects.archived` 列实现软删除。归档项目从活跃列表收起，会话不动不丢。永久删除才真销毁（可选会话转散落或连同删除）。

## 测试策略

| 层 | 框架 | 覆盖 |
|----|------|------|
| Rust 单元测试 | `cargo test --lib` | 420 tests: harness/commands/context/db |
| Rust 集成测试 | `cargo test` | provider SSE + memory e2e + message repo |
| 前端单元测试 | Vitest + happy-dom | 22 tests: utils/stores/bridge |
| 前端组件测试 | （计划中） | Combobox/ChatMessages 交互测试 |

### 测试命令

```bash
# Rust
cargo test --lib                          # 单元测试
SODIUM_LIB_DIR=... SODIUM_STATIC=true cargo test

# 前端
pnpm test                                 # vitest run
pnpm test:watch                           # vitest watch

# 类型 + Lint
pnpm typecheck                            # vue-tsc --noEmit
pnpm lint                                 # eslint
cargo clippy                              # Rust lint
```
