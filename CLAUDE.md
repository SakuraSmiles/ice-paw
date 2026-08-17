# CLAUDE.md — ice-paw 项目指引

## 项目概述
IcePaw — 本地优先的 LLM 对话工作站。Tauri v2 (Rust) + Vue 3 (TypeScript) 桌面应用。
当前版本：`0.3.6`。

## 设计规则（用户拍板，勿翻案）

**配置放置阶梯**——想新增任何配置项，先按序过四层，落不进 L1-L3 才考虑 L4：
- **L1 好默认**：能靠默认值 + 自适应解决的（如预算 3× 窗口、摘要自适应额度），根本不成为配置
- **L2 状态上屏**：用户需要「看见」的做成 HUD/胶囊/toast（如预算 pill），不是可编辑字段
- **L3 agent.yaml**：专家旋钮进 yaml +「打开 agent.yaml」入口 + 配置提案通道（propose_config_change / set_agent_yaml_field）
- **L4 表单**：只放出生证（name / id / model / key / workspace）

一句话：**状态上屏，配置进 yaml，表单只管出生证**。透明度问题（看不见）≠ 可配置性问题（不能改），勿用加表单字段治看不见——此为用户多次纠正的系统性偏差（最近一次 2026-08-16，P7 回滚 AgentForm 高级区）。

## 构建命令

### Rust
```bash
# cargo check（推荐）——需显式传 sodium 库路径
SODIUM_LIB_DIR="D:/workspace/ice-paw/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true \
cargo check --manifest-path packages/app/src-tauri/Cargo.toml

# cargo check --tests 验证测试编译
# cargo test --lib 本地可跑（comctl32 v6 manifest 已由 build.rs 注入 test harness）
#   ⚠️ 曾长期误记为「sodium DLL STATUS_ENTRYPOINT_NOT_FOUND」，真根因是 lib #[test]
#   harness 缺 Common-Controls v6 manifest（TaskDialogIndirect 静态导入），与 sodium 无关。
#   现状：684 passed / 0 failed（+ 集成测试：session_event_log_e2e 3、memory_e2e 3 等）
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

### 会话事件日志（session-event-log Phase 0+1 已 push 真机零 diff；Phase 2A 读路径切换已落地）
单一 append-only 事件日志基石（锁定愿景：统一 session / 多 agent 图协作 / 轨迹可还原）。
- **表**：migration 44 `session_events`（seq INSERT 子查询原子 + UNIQUE 兜底；message_id 故意无 FK——事件须活得比被删占位行久）
- **词表 13 kind** + typed emitters：`harness/event_log.rs`（EventCtx + warn-only 影子定位）
- **接线全退出路径**：chat_cmd/memory/loop_engine/cleanup(PersistOutcome)/retry_round/tool_executor/stages；**硬规则：事件 inline `.await` 禁 spawn，turn_ended 必须先于 cleanup() unregister 落库**
- **supersede**：自动续写同 message_id 多条 assistant_message，回放 last-wins
- **导出**：`export_session_trajectory` 命令 → JSONL（docs/backend-api-reference.md）
- **Phase 1 对账（889e9a8..0baa06c，6 commits）**：`harness/derive.rs` 纯回放（supersede last-wins / 空回退对称 / 坏 payload 记 issue 不吞）+ `harness/reconcile.rs`（A 侧 legacy 行提取走 `list_all_by_rowid` rowid 全量序 + 同一 `parse_content_blocks`；B 侧事件回放；turn 锚点走查分组）。diff 五类 MISSING_IN_DERIVED/MISSING_IN_LEGACY/CONTENT_MISMATCH/ORDER_MISMATCH/DERIVE_ISSUE = bug 清单；skipped 全部已文档化容忍（pre_phase0/epoch/incomplete_turn/error_row/discarded_row/empty_placeholder）。`reconcile_session` 命令只读出口。**真机验证：9a2a1968（20 事件 9 行）diffs=[] 且 skipped=[]；eae6d983（36 行零事件）全落 pre_phase0_no_events**。⚠️ 不变式：turn_id == user_msg_id；对账平面 = 行级原始形态（不跑 sanitize/投影）
- **Phase 2A 读路径切换（已随 0.3.6 发布）**：事件日志从影子升格为**干净会话的主读路径**。`harness/read_route.rs` 按会话路由：有事件 + 对账零 diff + 纯事件纪元 → **Derive**（`load_history_from_events` 派生 `Vec<MessageRow>`，锚回真 rowid，走与 legacy 完全相同的下游 Pipeline；派生输出与 legacy 同构同函数，reconcile 已证逐字节相等；摘要 `source_rowid` 取真 rowid 保连续性）。**指纹缓存** `(max_seq, max_rowid)` 追踪新数据（每轮刷新）；原地篡改不被察觉但活跃会话下轮即刷新、休眠会话不被读——`reconcile_session` 命令始终新鲜。诊断 `get_read_route_status` 命令。**不变式**：派生 MessageRow 必须能过 `load_history_with_window` 产出与 legacy 完全相同的视图（含 source_rowid）。
- **Phase 2B legacy 拼装退役（S1 阶段 1，2026-08-17）**：`load_history_from_events` 成为**唯一**生产读路径（session_runner 恒走派生）；`resolve()` 降级为健康监控——非绿（no_events / reconcile_diffs / mixed_epoch）时 error 日志后**照常派生**（历史可能缺行，不再静默回退 legacy；写路径 bug 不再被自动兜底静默吞掉）。排查走 `reconcile_session` / `get_read_route_status`；偏好回滚开关随分支删除（`session_read_path` 不在 KNOWN_KEYS，库内残留无害）。**回滚 = revert 阶段 1 commit**（messages 表双写持续，Legacy 拼装可整体恢复，零数据损失）。零事件会话行为锁定（e2e 场景 7）：派生空历史 + 回合照常完成 + boot backfill 自愈。
- **Phase 2B 前置 backfill（已落地 2026-08-17，3 commits，未 push）**：`harness/backfill.rs` boot 幂等扫尾——给零事件旧会话反向合成事件（reconcile 的逆函数：同 `parse_content_blocks`/空回退对称/同容忍清单 → 构造性零 diff → read_route 现有判据自动放行 Derive，read_route/derive/reconcile **零改动**）。范围只补零事件会话（混合纪元不补——seq 追加语义装不进历史前缀）；`turn_context` 不合成（payload 要 provider/model 快照，旧行没有，填当前配置=伪造）。actor=`backfill` 行是派生数据非运行时事实（append-only 边界的显式例外，重跑可删）；termination=`backfill` 诚实标注；created_at 直传行时间戳。**版本化重跑**：BACKFILL_VERSION 存 preferences，代码>库内 → 纯 backfill 会话删旧重写自愈；**冻结规则**：混入真实事件后永不可重写（会错序），frozen 仅计数。绕过 append_event 走 repo 批量（显式 seq/单会话事务/不广播）。7 测试：全形态零 diff+Derive green / 视图等价 / 幂等 / 混合纪元不碰 / 孤儿行降级 Legacy / 版本重跑 / 冻结。待真机验收：boot 日志 `[ice_paw.backfill]` 行 + 旧会话 `get_read_route_status` 变 Derive。
- Phase 2B 余项（S1 阶段 2/3）：summary `covered_until_rowid`→seq；Image base64 双份存储治理（legacy 拼装退役已落地，见上）

## 当前状态（2026-08-17）
- 版本 **0.3.6 已发布**（= 0.3.5 + UX 细节轮 + 模型配置重设计 + token 预算全分层修复 + S 批次结构减法 + backfill），main 与 origin 推平
- 分支：仅 `main`
- 近期递进：0.3.5 发版 → UX 细节轮 + 模型配置重设计 → 摘要链路三重治理 → S 批次结构减法（S2-S7 全清）→ **Phase 2B 前置 backfill 落地（00e9cb1 repo 原语 / 1def468 合成+接线 / 6eed139 版本化重跑+冻结）→ 0.3.6 发版**
- `cargo test --lib` 850 passed / 0 failed（+ 集成测试：session_runner_e2e 7、session_reconcile_e2e 6、session_event_log_e2e 3、memory_e2e 3、message_repo 5、provider 11）；clippy --tests -D warnings 0 警告
- 仍待办：**backfill 真机验收**（boot 日志 `[ice_paw.backfill]` 行 + 旧会话 `get_read_route_status` 变 Derive + reconcile 抽查零 diff）、0.3.5 发版手测（0.3.3 三重点 + Phase 2A 手测）、视觉适配/KB watcher/自动续写生产手测、proposal Phase 2（MCP 域）、S1 Phase 2B legacy 退役本体（前置全清待验收后动工）、S8 无限续写（待拍板）
