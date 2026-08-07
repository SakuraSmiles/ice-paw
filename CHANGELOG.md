# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本号遵循 [SemVer](https://semver.org/)。

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
