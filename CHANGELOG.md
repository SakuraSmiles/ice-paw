# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本号遵循 [SemVer](https://semver.org/)。

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
