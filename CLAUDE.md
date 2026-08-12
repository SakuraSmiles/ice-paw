# CLAUDE.md — ice-paw 项目指引

## 项目概述
IcePaw — 本地优先的 LLM 对话工作站。Tauri v2 (Rust) + Vue 3 (TypeScript) 桌面应用。
当前版本：`0.3.1`。

## 构建命令

### Rust
```bash
# cargo check（推荐）——需显式传 sodium 库路径
SODIUM_LIB_DIR="D:/workspace/ice-paw/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true \
cargo check --manifest-path packages/app/src-tauri/Cargo.toml

# cargo check --tests 验证测试编译
# ⚠️ cargo test 的 binary 无法启动（sodium DLL 运行时 STATUS_ENTRYPOINT_NOT_FOUND，已知问题）
#    编译 + 单测逻辑可验证，但 binary 无法执行；真测试靠 CI Linux
```

### 前端
```bash
pnpm run tauri:dev     # 开发模式（须在仓库根目录运行；端口 1420，被占时先 taskkill //F //PID <pid>）
pnpm run tauri:build   # 打包（须在仓库根目录运行；packages/app/ 下无此 script 会报 Missing script）
pnpm test          # vitest（本地可跑，前端重构主安全网）
pnpm typecheck && pnpm lint && pnpm build   # 不覆盖视觉/CSS 回归
```

## 架构概览

```
packages/app/src-tauri/src/
├── commands/         # Tauri 命令入口（chat/agent/conversation/mcp/kb/project/preferences/log/message）
├── harness/          # 核心业务逻辑
│   ├── mcp/          # MCP 工具系统（client trait/registry、内置工具、外部 server、bundled runtime、proposal_tool）
│   ├── provider/     # LLM provider 适配（anthropic/openai/mock + model_info 模型窗口表 + embedding）
│   ├── loop_engine.rs# 主循环调度（697 行，已拆出 loop/ 子模块）
│   ├── loop/         # 拆分出的子模块（context/events/reason/retry_round/stuck_detect/token_usage）
│   ├── tool_executor # 工具执行编排 + 授权流程
│   ├── proposal_guard.rs / proposal_registry.rs  # 配置提案 guardrail + 通道
│   ├── hooks.rs      # 对话钩子执行器（run_hooks + has_actions，4 接入点）
│   ├── kb/           # RAG 知识库（embedding/indexer/parser/watcher/ensure）
│   ├── budget.rs / summary_provider.rs / chat_state.rs / cleanup.rs / batch_writer.rs / oneshot_registry.rs / observable.rs
│   └── context/      # 上下文管道（token 估算、历史加载、摘要、裁剪阶段）
├── db/               # sqlx 数据层（models/repo/migrations）
├── infra/protocol.rs # 跨层事件 payload
└── lib.rs            # 启动入口（registry 初始化、MCP boot）
packages/app/src/
├── components/chat/  # 聊天 UI（ChatMessages, ConfigProposalCard, ToolAuthDialog...）
├── composables/      # 前端逻辑组合（useChatEvents, useThinkingTimer, useScrollFollow, useTheme, useNewConversation）
├── stores/           # Pinia 状态管理（chat, agent, project）
├── api/bridge.ts     # Tauri invoke 统一入口
└── types/index.ts    # 前端类型定义
```

## 关键系统

### 配置提案系统（Phase 1，已 commit a4f0e5f + push；agent.yaml 写保护加固 132cf19）
agent 调用 `propose_config_change` 工具提出创建/修改 agent 提案 → 前端渲染审批卡片 → 用户批准后前端走现有可信 Tauri 命令应用。**agent 全程无写权限**。
- `proposal_tool.rs`(mcp/) + `proposal_guard.rs` + `proposal_registry.rs`(harness/) + `ConfigProposalCard.vue`
- guardrail：🔴红线→Err，🟡→Medium，🟢→Low；API Key 走引用槽位（`key_slot:"__SLOT__"`），用户在卡片亲手填
- 安全加固(132cf19)：写工具 `reject_sensitive()` 拦硬写 agent.yaml + `register_meta_tools()` 强制注入合法通道

### MCP 工具系统
- `McpClient` trait：name/description/parameters/execute/execute_with_context
- `McpRegistry`：RwLock<HashMap<String, Arc<dyn McpClient>>>
- `McpServerManager`：统一 Server Pool 状态机（Disabled→Starting→Running/Failed），启动时后台并行启动
- 内置 MCP runtime：内置 Node + 预打包包（runtime_kind 列）；当前 2 包（builtin-thinking + builtin-memory，filesystem 已于 v0.2.5 下线）
- 文件工具 native 化：`file_tools.rs`（Write/Edit/Delete/Move/CreateDirectory），不再依赖外部 filesystem server

### 对话钩子系统（已 commit 1c2a1d8 + push）
- 4 接入点：ConversationStart(chat_cmd) / BeforeLlm(loop_engine) / AfterTool(tool_executor) / ConversationEnd(loop_engine)
- 内置动作：InjectPrompt/CallTool/Log
- 配置在 agent.yaml `hooks` 字段（AgentFileConfig.hooks）

### 上下文预算（Phase 0+1+2，已 commit push）
- 真实 token 估算（覆盖 tool_use/tool_result/thinking/image 块）+ per-agent context_window
- TokenWindowStage（max_input_tokens 的 80% 硬裁历史）
- Phase 2 滚动增量摘要（covered_until_rowid 追踪 + fold 55%·40%）

### RAG 知识库（已 commit push）
- KB 文档 → embedding → 索引 → 语义检索（search_kb 工具）；watcher 自动索引；产品帮助种子已落地

### 工具名合规化（deepseek 400 双修，commit a02a7b0，已 push origin/main）
- migration 39 `tool_index` 列 + `t{idx}_` 命名 + 历史 sanitize（修工具名违反 `^[a-zA-Z0-9_-]+$`）
- OpenAI 适配层 `chat_message_to_openai` 1→N 展开 tool_result 为多条 role=tool（OpenAI-only，Anthropic 零改）

### 大文件拆分（已 ff-merge 到 main 17b1ffc，分支已删）
- loop_engine 1343→697 + 抽 `loop/` 子模块；chat.ts 843→532（抽 useChatEvents）；Sidebar/ChatMessages 抽 composables

### 视觉能力统一适配（事2 / 方案 C，bfcd2ce + 2ce76cb + f054e38 + c10d02e，未手测）
4 个 Image 块注入入口统一走"按有效视觉能力适配"，杜绝向非视觉模型塞 Image（→400/"看不到"）：
- **能力探测**：`provider/model_info.rs::effective_supports_vision(agent.supports_vision, provider, model)`——OR 关系（agent 显式 =1 权威；=0 按模型表自动探测，如 MiniMax-M3）。零 schema 改动。
- **统一适配**：`harness/modal.rs`——`gather_vision_candidates`（DB 收集凭据：显式 vision 配置→agent 自带视觉模型→GLM 视觉 MCP env）/ `adapt_blocks_for_vision`（有效视觉原样过；非视觉逐图代读成 Text、失败剥离+诚实提示）/ `strip_image_blocks_to_marker`（历史静默剥离）。
- **4 入口接线**：① 用户上传+③ 历史 → `context/stages.rs::ModalCapabilityStage`（Pipeline，TokenWindow 后 Final 前）；② 工具返图 → `tool_executor` 注入 Image 前查 effective_vision（时序独立于 Stage——工具循环后续轮次的图进不了 Stage，必须在此守卫）；④ `view_attachment_image` 判断改 effective_supports_vision + 凭据收集复用 gather。
- **⚠️ 不变式**：任何新增的 Image 块注入点都必须经 `effective_supports_vision` / `adapt_blocks_for_vision`，不得对非视觉模型直塞 Image。

## 当前状态（2026-08-12）
- 版本 **0.3.1**（已打包 NSIS 231M + MSI 244M；0.3.0 生产闪退热修）。main 本地领先 origin/main 多个 commits（push 待指示）
- 分支：仅 `main`（refactor/split-bigfiles-composable + frontend-rewrite + immersive-mode 三分支 local/remote 已删）
- 近期递进：17b1ffc（大文件拆分+孤儿 tool_use 根治）→ ec08e17（治本②③KB watcher+自动续写）→ 0.2.9 打包 → 69d2163（Phase B 视觉读取 14 commits，0.3.0）→ 1e5868c（migration checksum 自愈，热修 0.3.0 闪退）→ 409324e（0.3.1）→ **bfcd2ce/2ce76cb/f054e38/c10d02e（视觉能力统一适配：事1 认知修复 + 事2 方案 C，4 图片入口全接通 modal.rs + ModalCapabilityStage，未手测）**
- 测试 binary 无法运行（sodium DLL，STATUS_ENTRYPOINT_NOT_FOUND），`cargo check`/`--tests` 验证编译；真测试靠 CI Linux；前端 vitest 本地可跑
- 仍待办：视觉能力统一适配真机手测（非视觉 agent 代读 / M3 自动视觉 / 视觉 agent 不变）、KB watcher + 自动续写生产手测、proposal Phase 2（MCP 域）、可测试性（sodium DLL 是门控钥匙）
