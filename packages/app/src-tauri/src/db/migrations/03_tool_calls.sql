-- =========================================================================
-- IcePaw 数据库迁移 V3：工具调用支持（P2-1）
-- 来源：icepaw-p0-p2-plan.md §2.1.3 P2-1a
-- 说明：
--   1) messages 表新增 content_blocks 列（JSON 数组，默认 '[]'）
--      - 双写策略：content (TEXT) 保留兼容旧消息，content_blocks 存完整结构
--      - 旧消息 content_blocks 为空数组，读取时回退到 content 字段
--   2) 新增 tool_calls 表（工具调用审计日志，可选使用）
-- =========================================================================

-- messages 表新增 content_blocks 列
ALTER TABLE messages ADD COLUMN content_blocks TEXT NOT NULL DEFAULT '[]';

-- 工具调用记录表（调试和审计用）
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    arguments TEXT NOT NULL,          -- JSON 字符串
    result TEXT,                       -- JSON 字符串，执行结果
    is_error INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_tool_calls_message ON tool_calls(message_id);
