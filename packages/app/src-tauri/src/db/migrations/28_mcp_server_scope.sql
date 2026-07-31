-- =========================================================================
-- IcePaw 数据库迁移 V28：MCP Server scope（隔离级别）
-- per-agent server 架构：global 全局共享 / per_agent 按 agent 隔离启动
-- global：lib.rs setup 启动，所有 agent 共享（无路径 server，如 web search）
-- per_agent：send_message 时按 agent 启动，args 的 {workspace} 替换为该 agent workspace
-- =========================================================================

ALTER TABLE mcp_servers ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
