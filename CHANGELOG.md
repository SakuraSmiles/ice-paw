# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本号遵循 [SemVer](https://semver.org/)。

## [Unreleased]

## [0.3.7] — 2026-08-17

> 从 0.3.6 以来的主要调整：S1——session_events 升格唯一读路径（legacy 拼装退役）+ 摘要锚点 seq 化 + Image 双份存储治理；真机验收五项全绿。

### Changed
- **legacy 拼装退役（S1 阶段 1）**：事件派生 `load_history_from_events` 成为唯一生产读路径（恒 Derive）；read_route 降级为健康监控——非绿（无事件/对账差异/混合纪元）记 error 日志后**照常派生**，写路径 bug 不再被自动回退兜底静默吞掉。messages 表双写持续保留为回滚底座（revert 阶段 1 commit 可整体恢复，零数据损失）。
- **摘要锚点 seq 化（S1 阶段 2）**：migration 46 `covered_until_seq`（被覆盖消息首现事件 seq，与派生排序位严格一致）+ 存量回填；摘要状态双写双读，锚点定位 seq 优先 rowid 兜底——根治 messages 表无 AUTOINCREMENT 的 rowid 复用漂移风险，旧事件零迁移。
- **Image 双份存储治理（S1 阶段 3）**：消息类事件 payload 的 Image 块改轻量引用 `image_ref`（payload v2，字节只在 messages 行）；写侧唯一入口 `refify_blocks`，读侧三路水合（LLM 视图 / 对账 / 前端轨迹与导出），未命中诚实降级文本标记不静默消失；BACKFILL_VERSION=2 纯 backfill 会话 boot 自动重写自愈，v1 内联旧事件永久可读。真机实测：两张图的事件 payload 从潜在 4.7MB 双写降至 326 字节。
- **S1 真机验收五项全绿**：backfill（9 会话 824 事件零失败）／恒 Derive（当日路由决策全 green 零 diff）／发图 payload 无 base64（模型回复描述画面 = 水合实证）／摘要折叠 `covered_until_seq` 落值／轨迹检查器图片 v1/v2 两形态显示正常。

### Fixed
- 终止原因文案收敛 `utils/termLabels` 单一真相源——`backfill` 补「历史补录」标注 + 非异常化呈现。

## [0.3.6] — 2026-08-17

> 从 0.3.5 以来的主要调整：UX 细节轮收官、模型配置重设计、token 预算全分层修复、S 批次结构减法、旧会话事件 backfill。

### Added
- **模型配置重设计（Provider 注册表单一真相源）**：后端 `PROVIDERS` 9 条目录元数据供前后端共用 + `list_providers` / `test_provider_connection` 命令（测试连接与拉取模型合一，一次往返两用）；前端模型选择改 GroupedSelect 分组下拉（Provider 品牌图标、组头不可选）→ combobox 可选可输（手输目录外名字落自定义）；预设厂商 URL 锁定只读，智谱双端点 `alt_urls` 自动匹配固化；空 Key 按 provider 目录判定放宽（Ollama 本机无需 Key）。
- **UX 细节优化清单 12 项 + 修复轮**：审批重做——按注意力路由（输入区上方/消息流内分层）+ 分层授权记忆；可调面板宽度 + 记忆 + 规范化管理；轮次导航条 v2（定容滑动窗口，N/M 徽标跨位不漂移）+ 任务胶囊深化；全局过渡动画统一「淡入+微升」+ `prefers-reduced-motion` 兜底；委派标题去前缀 + agent 名徽标；项目快速新建；头部操作外置。
- **token 预算全分层修复**：摘要自适应额度 4096→16384（连续空结果翻倍、成功回落、3 连空触发熔断）；预算可观测——`chat:budget` 事件 + 预算 pill HUD（≥80% warn 态）+ 续期 toast，终止文案带数字与指引；agent.yaml 定向改写命令（`get_agent_yaml_fields` / `set_agent_yaml_field`，白名单键 + 写前重解析校验 + 原子写）。
- **旧会话事件 backfill（session-event-log Phase 2B 前置）**：boot 幂等扫尾——给零事件旧会话反向合成 `session_events`（reconcile 的逆函数：同 parser / 同空回退 / 同容忍清单 → 构造性零 diff → read_route 自动路由 Derive）；`turn_context` 不合成（旧行无 provider/model 快照，不伪造）；actor=`backfill` 行是派生数据可重跑，termination=`backfill` 诚实标注，created_at 直传行时间戳；版本化重跑自愈（BACKFILL_VERSION 落 preferences，代码>库内 → 纯 backfill 会话删旧重写）+ 冻结规则（混入真实事件后永不可重写）。
- **send_message 全链路 e2e（S5）**：`session_runner_e2e` 六场景（正常 / 空响应 / 限流退避中取消 / 显式预算触顶 / 流中取消占位 discard / 工具轮配对），MockProvider `ToolCallThenText` 驱动，断言消息行 + 事件序 + UI 瞬态事件 + TurnSummary 四层。

### Changed
- **S 批次结构减法（测试数不降硬约束）**：S2 `protocol.rs` 1161 行拆 `protocol/` 目录（llm / input / events，全库导入零改）；S3 chat_cmd 附件机器整体迁 `harness/attachments.rs`（695→~290 行回归编排门面）；S4 LoopConfig 数据袋（auth 运行时件挪 LoopContext、`StreamLoopInput` 成袋删超长签名）；S6 主循环链去 AppHandle 硬依赖——`LoopEmitter` trait + 七模块换装（瞬态 UI 进度与可回放事实两通道分明）；S7 `tool_trim_threshold` 废弃字段全链摘除（schema/repo/命令/前端，serde 容忍旧 yaml）。
- **摘要链路治理**：stream_summary 走默认方法 + GLM 摘要请求注入 `thinking:disabled` + 连续空结果熔断（3 次 10min）；MemoryStage Err 降级不阻塞回合。

### Fixed
- **GLM thinking 烧光摘要额度 → 空摘要 → 历史永不折叠 → 每轮全量重发触顶**：三重治理 + 摘要锚点 SQL 排空占位行 + IO 视位修复；轮次导航条双修（视位冻结）。
- **崩溃后 turn 永远「进行中」死数据**：boot 补记未闭合 turn 的 `turn_ended(interrupted)`（幂等扫尾，历史脏数据自动治好）。
- **父会话委派卡泄漏进子会话**（跨会话流式态复位收敛单一入口）、删除复活竞态、审批卡宽度对齐输入框。
- **workspace 路径判定**：`infra/path_norm` 共享归一判定；前端 DB 时间解析归一 `parseDbTime` + 侧栏后台刷新骨架屏闪现。
- **模型拉取 401 撞脸**：无 Key 短路引导 + 错误翻译 + 存量 Key 不跨 provider 混用。
- CI Linux 编译：首启窗口尺寸的 `hwnd()` 调用补 `cfg` 门。

## [0.3.5] — 2026-08-15

> 从 0.2.7 到 0.3.5 的主要功能调整（合并概括，未逐小版本拆分）。

### Added
- **会话事件日志与轨迹视图**：基于 migration 44 `session_events` 表的 append-only 事件日志基石，统一 session / 多 agent 图协作 / 轨迹可还原。词表 13 kind + typed emitters，事件 inline `.await` 禁 spawn，turn_ended 必须先于 cleanup() unregister 落库；supersede 机制让同一 `message_id` 的多次 assistant_message 自动续写，回放 last-wins；导出命令 `export_session_trajectory` → JSONL。
- **会话事件对账与派生回放（Phase 1）**：`harness/derive.rs` 纯回放（supersede last-wins / 空回退对称 / 坏 payload 记 issue 不吞）+ `harness/reconcile.rs` A 侧 legacy / B 侧事件回放 / turn 锚点走查分组，对账平面 = 行级原始形态。`reconcile_session` 命令只读出口。
- **事件日志读路径切换（Phase 2A）**：`harness/read_route.rs` 按会话路由——有事件 + 对账零 diff + 纯事件纪元 → Derive（`load_history_from_events` 派生 `Vec<MessageRow>`，锚回真 rowid），其余 → Legacy；零风险（派生输出与 legacy 同构同函数）；指纹缓存 `(max_seq, max_rowid)` 追踪新数据；偏好 `session_read_path=legacy` 一键回滚；诊断命令 `get_read_route_status`。
- **文件系统工具集 native 化**：bundled filesystem server（`@modelcontextprotocol/server-filesystem`）下线，6 个核心工具与内置 native 重复，删除；其独有 5 项补成 native 内置工具，授权统一为 `PathWhitelist`：`directory_tree`（递归目录树，跳过 .git/node_modules，限深度 8/节点 2000）、`move_file`（移动/重命名，跨卷回退 copy+delete，源文件自动备份）、`create_directory`（建目录含父目录，幂等）、`get_file_info`（文件元信息）、`read_multiple_files`（批量读 ≤20 文件，单文件 >1MB 跳过）。
- **配置提案 Guardrail（Phase 1）**：`propose_config_change` 工具 + `proposal_guard.rs` + `proposal_registry.rs`，agent 全程无写权限。Guardrail 三档分级：🔴 红线（删除/跨 agent/api_key 非占位符）→ 拒绝；🟡 敏感（带工具 / enabled_tools 变更）→ Medium；🟢 非敏感（名称/温度/system_prompt）→ Low。API Key 走引用槽位 `__SLOT__`，用户在审批卡片亲手填。写保护加固：`reject_sensitive()` 拦硬写 agent.yaml + `register_meta_tools()` 强制注入合法通道。
- **视觉能力统一适配**：4 个 Image 块注入入口统一走"按有效视觉能力适配"，杜绝向非视觉模型塞 Image。`provider/model_info.rs::effective_supports_vision`（OR 关系：agent 显式 =1 权威，=0 按模型表自动探测）；`harness/modal.rs` 统一 `gather_vision_candidates` / `adapt_blocks_for_vision` / `strip_image_blocks_to_marker`；4 入口接线（用户上传 / 工具返图 / 历史 / `view_attachment_image`）。
- **上下文预算与滚动增量摘要**：真实 token 估算（覆盖 tool_use/tool_result/thinking/image 块）+ per-agent `context_window`；`TokenWindowStage`（max_input_tokens 的 80% 硬裁历史）；Phase 2 滚动增量摘要（`covered_until_rowid` 追踪 + fold 55%·40%）。
- **多 Agent 委派与 Loop 拆分**：loop_engine 1343→697，抽 `loop/` 子模块（context/events/reason/retry_round/stuck_detect/token_usage）；chat.ts 843→532（抽 useChatEvents）；Sidebar / ChatMessages 抽 composables。
- **工具名合规化与 OpenAI 适配**：migration 39 `tool_index` 列 + `t{idx}_` 命名 + 历史 sanitize（修工具名违反 `^[a-zA-Z0-9_-]+$`）；OpenAI 适配层 `chat_message_to_openai` 1→N 展开 tool_result 为多条 `role=tool`（OpenAI-only，Anthropic 零改）。
- **内置 WebView2 离线安装器**：Windows 安装包改用 `offlineInstaller` 模式，把 WebView2 Runtime 离线安装器嵌入 MSI/NSIS，纯净 Windows 双击即装。

### Changed
- **内置工具清单动态化**：设置页「内置工具」由后端 `list_builtin_tools` 命令 + `register_builtin` 单一事实来源驱动，前端动态拉取；中文描述降级为本地化文案层，缺失回退后端原文。
- **内置 MCP 运行时**：3 个轻量内置 server（sequential-thinking / memory / filesystem）从 npx 运行时拉取改为安装包自带 Windows-x64 Node + 预打包 `node_modules`，运行时零网络、零系统 node 依赖；filesystem 已随 native 化下线，bundled runtime 仅保留 thinking / memory。

### Fixed
- **外部 MCP 工具调用分发**：`ExternalToolProxy` 把带 `server_name.` 前缀的工具名原样发给 server 导致 -32602，拆成 `name`（带前缀，LLM 侧）+ `server_tool_name`（原始，发 server）两字段。
- **CI 修复**：bundled runtime 起 CI 红——`tauri-build` 在编译期校验 `bundle.resources`，CI 改为创建占位 resources 让校验通过；顺带修 4 处潜伏 clippy 违规。
- **错误横幅跨会话串味**：A 会话出错后切到 B 会话顶部仍显示 A 的错误 → 按 `conversation_id` 隔离（Map + computed）。
- **filesystem server 包名 404**：`@anthropic-ai/mcp-server-filesystem` 已下架 → `@modelcontextprotocol/server-filesystem`。
- **命令行窗口闪现**：Windows 上 `run_command`/`git` 等工具控制台一闪而过 → 统一 `CREATE_NO_WINDOW`。
- **工具打分历史权重失效**：`tool_calls` 空壳表导致 scoring 维度从未生效 → 随审计接入自动恢复。
- 切会话不丢卡片、取消通道、emit→invoke 修正、thinkingTimer KeepAlive 生命周期修复、工具授权弹窗背景点击不再误触拒绝、P0 稳定性修复（crypto Mutex 毒化 / spawn token 残留 / reqwest `expect` 崩溃 / 前端事件监听器泄漏 / TS 预存错误）。

## [0.2.7] — 2026-08-07

### Changed
- **内置 WebView2 离线安装器**：Windows 安装包改用 `offlineInstaller` 模式（`bundle.windows.webviewInstallMode`），把微软 WebView2 Runtime 离线安装器嵌入 MSI/NSIS。纯净 Windows（无 WebView2、无网络）双击即装，彻底告别「缺少 WebView2 Runtime」报错。代价：安装包体积 MSI 41M→241M、NSIS 26M→229M（+约 200MB，微软该安装器实际体积，比 Tauri 源码注释里的 127MB 旧值大）。安装器由 Tauri 打包时自动从微软 CDN 下载并嵌入（编译期不校验文件存在，故 CI 无需为其建占位）；`offlineInstaller` 在 Tauri v2 schema 中**不接受 `path` 字段**（仅 `silent`），与部分教程说法相反。

## [0.2.6] — 2026-08-07

### Fixed
- **内置工具清单动态化**：设置页「内置工具」原为前端硬编码数组，与后端 `register_builtin` 漂移——0.2.5 新增 5 个 native 文件工具时漏改前端，导致设置页一直少显示（工具实际可用，仅展示漏，非数据库问题）。改为后端新增 `list_builtin_tools` 命令（复用 `register_builtin` 单一事实来源）+ 前端动态拉取，计数与清单永远反映真实，新增工具零漂移；中文描述降级为本地化文案层，缺失回退后端原文。
- **CI 修复**：0.2.4 bundled runtime 起 CI 一直红——`tauri-build` 在编译期校验 `bundle.resources`（node.exe / node_modules）存在，但这些由 `prepare:mcp` 下载、gitignore 不入库，CI 从未 prepare，build script 在 cargo check 阶段就炸。CI 改为创建占位 resources 让校验通过（CI 只验编译、不产出安装包，真实打包仍走 `beforeBuildCommand` 的 `prepare:mcp`）。顺带修 4 处潜伏 clippy 违规（此前 CI 在 check 就死，从未暴露）。

## [0.2.5] — 2026-08-07

### Changed
- **移除「工程专家团队」内置工具集**：价值低、依赖系统 node + npx 联网拉取，不再随产品内置。已安装用户的旧记录由 migration 36 自动清除。
- **「文件系统工具集」整合为 native 内置工具**：原 bundled MCP Server（`@modelcontextprotocol/server-filesystem`）的 6 个核心工具与内置 native 工具完全重复且内置更优（自动备份 / 唯一性校验 / 大文件分页 / 噪音目录过滤），予以移除；其独有的 5 个能力补成 native 内置工具，零 node 进程开销、授权模型统一为 `PathWhitelist`：
  - 新增 `directory_tree`——递归目录树（跳过 .git/node_modules 等，限深度 8 / 节点 2000）。
  - 新增 `move_file`——移动 / 重命名（跨卷回退 copy+delete，源文件自动备份）。
  - 新增 `create_directory`——建目录含父目录（幂等）。
  - 新增 `get_file_info`——文件元信息（大小 / 类型 / 只读 / 修改·创建·访问时间）。
  - 新增 `read_multiple_files`——批量读 ≤20 文件（单文件 >1MB 跳过；多路径无法自动放行故每次确认）。
  - `extract_path_from_args` 扩展支持 `source`/`destination`，使 `move_file` 可经 source 走白名单授权。
- 内置 MCP 运行时不再打包 `@modelcontextprotocol/server-filesystem`（thinking / memory 仍需保留 node runtime）。

## [0.2.4] — 2026-08-07

> 0.2.x 线的 beta 阶段第 4 个迭代。

### Added
- **内置 MCP 运行时**：3 个轻量内置 server（sequential-thinking / memory / filesystem）从 npx 运行时拉取改为安装包自带 Windows-x64 Node + 预打包 `node_modules`，运行时零网络、零系统 node 依赖。修复生产 0.2.3 上「深度推理」因 npx 缓存缺传递依赖 `zod` 启动失败。Playwright/maifady 维持 npx。

### Fixed
- **外部 MCP 工具调用分发**：`ExternalToolProxy` 原把带 `server_name.` 前缀的工具名（如 `深度推理.sequentialthinking`）原样发给 server，server 只认原始名 → JSON-RPC -32602 "Tool ... not found"。proxy 拆成 `name`（带前缀，LLM 侧）+ `server_tool_name`（原始，发 server）两字段。潜伏 bug，影响所有外部 MCP 工具调用（非 bundled 专属）。
- **错误横幅跨会话串味**：`lastError` 原为全局 ref，A 会话出错后切到 B 会话顶部仍显示 A 的错误。改为按 conversation_id 隔离（Map + computed）。
- **filesystem server 包名 404**：`@anthropic-ai/mcp-server-filesystem` 已下架，随 bundled 运行时迁到 `@modelcontextprotocol/server-filesystem`。

## [0.2.3] — 2026-08-07

> 0.2.x 线的 beta 阶段第 3 个迭代。

### Added
- **工具调用审计**：`tool_calls` 表接入 `tool_executor`，每次工具调用记录 tool_name/arguments/result/is_error/耗时/起止，可回溯 agent 行为与排查慢命令。

### Fixed
- **命令行窗口闪现**：Windows 上 agent 调用 `run_command`/`git` 等工具时控制台窗口一闪而过（统一 `CREATE_NO_WINDOW`）。
- **工具打分历史权重失效**：`tool_calls` 空壳表导致 `scoring` 的「最近调用加权」维度从未生效，随审计接入自动恢复。

## [0.2.2] — 2026-08-06

> 0.2.x 线的 beta 阶段第 2 个迭代（对应原计划的 beta.2）。自 [0.2.0-beta.1] 起，改用 patch 位编码迭代号，版本号统一为纯数字（MSI 兼容）。

### Added
- **Agent 代配置（提案模式）Phase 1**：`propose_config_change` 工具，LLM 从对话中提出创建/修改 agent 提案，用户审批后生效。Guardrail 校验层（🔴 红线永久拒绝）。前端审批卡片（字段全展开 + API Key 安全输入）。

### Fixed
- **MiniMax 2013**：`sanitize_history` 丢弃孤儿 tool_use 与空消息占位、合并连续同角色消息；LLM 400 错误诊断增强（8421f13）。
- **P0 稳定性修复**：crypto Mutex 毒化、spawn token 残留、reqwest `expect` 崩溃、前端事件监听器泄漏、TS 预存错误（531d6a2、dcfc6ab）。
- thinkingTimer KeepAlive 生命周期：切会话后定时器不再泄漏/错乱（159cc9b）。
- 工具授权弹窗背景点击不再误触「拒绝」（80290fe）。
- **审批/授权可靠性**：切会话不丢卡片、取消通道、emit→invoke 修正（a4f0e5f）。

### Changed
- CI 修复：Phase 1 引入的测试编译错误与前端 lint（734a01f、1e49a43）。

## [0.2.0-beta.1] — 2026-08-05

### Added
- **对话钩子系统**：4 个生命周期接入点（ConversationStart/BeforeLlm/AfterTool/ConversationEnd）+ 3 个内置动作（InjectPrompt/CallTool/Log），配置在 agent.yaml。
- **产品帮助知识库**：6 篇中文帮助文档种子到全局 KB，agent 可通过 search_kb 自服务检索。
- **RAG v2 语义检索修复**：修配置读取 bug + 召回阈值 + RRF 混合检索 + 切换模型自动重建向量 + 可观测性 UI。
- **MCP 架构重设计**：统一 Server Pool 状态机，启动不阻塞，前端简化。
- 项目归档/移动会话/双层 Option workspace_path/N+1 修复/activeProjectId 校验

## [0.1.0-beta.1] — 2026-08-02

### Added

- OpenAI / Anthropic / 智谱 GLM / DeepSeek / MiniMax 多 Provider 支持
- Agent 管理：创建、编辑、删除，独立配 provider/model/system prompt/temperature
- `agent.yaml` 文件配置：放在 agent workspace 里自动读取
- 会话管理：新建、重命名、置顶、删除、搜索、分页加载
- 流式聊天：Markdown 渲染、代码高亮、thinking 和 tool_call 展开
- 消息复制、图片粘贴、链接外链打开
- 项目空间：创建、编辑、切换、归档/恢复、永久删除
- 项目内成员管理
- MCP 内置工具：read_file、write_file、edit_file、list_directory、search_files、run_command、git、web_fetch、search_kb、read_kb_document、save_to_kb、read_agent_config
- 外部 MCP Server：stdio JSON-RPC 连接，global/per_agent scope，trusted/untrusted 权限
- 知识库（RAG v1）：文件自动索引、语义检索、agent/项目/全局三级 scope
- API Key Stronghold 加密存储
- 统一时区系统（设置页改时区即时生效）
- 暗色模式
- 日志查看页（daily rotate 持久化）
- 420 Rust tests + 31 前端 tests

### Changed

- chat 模块从 1568 行单体拆为 context pipeline（Template → OS → SystemPrompt → History → Memory → Final）
- LLM provider 抽象为 trait + 多 adapter 模式
- 引入 LoopBudget + RetryState 替代硬编码常量
- 项目卡片改为内联编辑
- 会话切换不再重挂载组件（消除闪屏）

### Fixed

- 会话卡死：Pipeline 中途失败时 conv_id 永久残留 → scopeguard RAII 守卫
- MCP env 泄漏：子进程继承全部环境变量 → 白名单过滤
- 流式生成中切走再切回内容丢失 → bgStreams 快照
- 侧栏 >30 天旧日期截 UTC 时区错误 → 统一时间系统
- 浅色主题暗色气泡不可读 → tint token 统一
- Base URL path 被截断、MiniMax 400 错误、finish_reason 泄漏等多处小修
- 22 处 dead_code → 清理至 3 处（均为有意保留）
