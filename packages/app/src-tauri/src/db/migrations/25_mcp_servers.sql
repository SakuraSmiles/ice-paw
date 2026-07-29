-- =========================================================================
-- IcePaw 数据库迁移 V25：MCP Server 配置持久化
-- Phase 2: 外部 MCP Server 连接器
-- 每次对话使用；非 NULL 表示继承 Agent 配置，不覆盖。
-- =========================================================================

-- MCP Server 配置表
CREATE TABLE IF NOT EXISTS mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    command TEXT NOT NULL,
    args TEXT NOT NULL DEFAULT '[]',            -- JSON 数组
    env TEXT DEFAULT '{}',                      -- JSON 对象，可选环境变量
    enabled INTEGER NOT NULL DEFAULT 1,
    trust_level TEXT NOT NULL DEFAULT 'untrusted',  -- trusted | untrusted
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
