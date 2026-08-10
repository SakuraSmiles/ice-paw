-- ============================================================================
-- IcePaw Migration 40: MCP Server transport / url / headers（远程 MCP 传输）
-- ----------------------------------------------------------------------------
-- 背景：此前 mcp_servers 只能描述 stdio 子进程（command/args/env）。要接入
--       GLM Coding Plan 等远程 MCP Server（streamable HTTP / SSE），需要
--       传输类型 + URL + 自定义请求头三列。
--
-- 设计：
--   - transport TEXT NOT NULL DEFAULT 'stdio'
--       'stdio' → 本地子进程（默认，所有现有 server 自动归此）
--       'http'  → 远程 streamable HTTP（POST JSON-RPC）
--       'sse'   → 远程 SSE（GET 长连接 + POST，旧式）
--       transport 是顶层路由；runtime_kind 仅在 transport='stdio' 时有意义（保留原列）。
--   - url TEXT —— 远程 server 端点（stdio server 为 NULL）
--   - headers TEXT —— JSON 对象，远程 server 自定义请求头（如 Authorization）
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 引入新列；老 server transport 自动 'stdio'。
--   - 全新安装：本 migration 跑时 mcp_servers 表尚空，UPDATE 命中 0 行；
--     随后 seed_defaults 播种的 builtin 也会得到 transport='stdio'，两路径收敛。
--   - sqlx::migrate! 在 db init 执行（早于 seed_defaults），按文件名执行不重复。
-- ============================================================================

ALTER TABLE mcp_servers ADD COLUMN transport TEXT NOT NULL DEFAULT 'stdio';
ALTER TABLE mcp_servers ADD COLUMN url TEXT;
ALTER TABLE mcp_servers ADD COLUMN headers TEXT;

-- 兜底：确保旧行 headers 非 NULL（解析时再 serde 兜底到 {}，这里仅防 NULL）
UPDATE mcp_servers SET headers = '{}' WHERE headers IS NULL;
