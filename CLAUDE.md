# CLAUDE.md — ice-paw 项目指引

## 项目概述
IcePaw — 本地优先的 LLM 对话工作站。Tauri v2 (Rust) + Vue 3 (TypeScript) 桌面应用。
当前版本：`0.2.4`，分支 `main`。

## 构建命令

### Rust
```bash
# cargo check（推荐，无需链接 libsodium）
SODIUM_LIB_DIR="D:/workspace/ice-paw/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true \
cargo check --manifest-path packages/app/src-tauri/Cargo.toml

# cargo test（binary 可能因 sodium DLL 无法启动，已知问题）
# 同上加 SODIUM_LIB_DIR + SODIUM_STATIC
```

### 前端
```bash
pnpm tauri:dev     # 开发模式（端口 1420，被占时先 taskkill）
pnpm tauri:build   # 打包
```

## 架构概览

```
packages/app/src-tauri/src/
├── commands/         # Tauri 命令入口（chat_cmd, agent_cmd, mcp_cmd...）
├── harness/          # 核心业务逻辑
│   ├── mcp/          # MCP 工具系统（client trait, registry, 内置工具, 外部 server）
│   ├── provider/     # LLM provider 适配（anthropic, openai）
│   ├── loop_engine   # 对话循环（发送→流式→工具→下一轮）
│   ├── tool_executor # 工具执行编排 + 授权流程
│   ├── proposal_guard     # 配置提案 guardrail（新增，未 commit）
│   └── proposal_registry  # 提案响应通道（新增，未 commit）
├── db/               # sqlx 数据层
├── infra/protocol.rs # 跨层事件 payload
└── lib.rs            # 启动入口
packages/app/src/
├── components/chat/  # 聊天 UI（ChatMessages, ConfigProposalCard...）
├── stores/           # Pinia 状态管理（chat, agent, project）
├── api/bridge.ts     # Tauri invoke 统一入口
└── types/index.ts    # 前端类型定义
```

## 关键系统

### 配置提案系统（Phase 1，新增，未 commit）
agent 调用 `propose_config_change` 工具提出创建/修改 agent 提案 → 前端渲染审批卡片 → 用户批准后前端走现有可信 Tauri 命令应用。agent 全程无写权限。
- 新增 4 文件：`proposal_tool.rs`, `proposal_guard.rs`, `proposal_registry.rs`, `ConfigProposalCard.vue`
- 修改 13 文件：protocol.rs, client.rs, tool_executor.rs, lib.rs, chat.ts, ChatMessages.vue 等

### MCP 工具系统
- `McpClient` trait：name/description/parameters/execute/execute_with_context
- `McpRegistry`：RwLock<HashMap<String, Arc<dyn McpClient>>>
- `McpServerManager`：统一 Server Pool 状态机（Disabled→Starting→Running/Failed）
- 启动时后台并行启动所有 enabled MCP Server

### 对话钩子系统
- 4 个生命周期接入点：ConversationStart/BeforeLlm/AfterTool/ConversationEnd
- 内置动作：InjectPrompt/CallTool/Log
- 配置在 agent.yaml `hooks` 字段

## 当前状态（2026-08-05）
- HEAD: `53b9575`（KB 帮助），领先 origin/main 2 commits
- 钩子系统：已 commit push（1c2a1d8），待端到端手测
- Agent 代配置 Phase 1：17 文件未 commit，端到端验证通过
- 测试 binary 无法运行（sodium DLL），cargo check 可验证编译
