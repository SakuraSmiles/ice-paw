-- =========================================================================
-- IcePaw Migration 20: agents.avatar（REQ-AGENT-002）
-- ----------------------------------------------------------------------------
-- 用途：
--   给 agents 表增加 avatar 字段（用户上传的自定义头像，存为 base64 字符串）。
--   - NULL = 未上传，使用 AgentMeta 派生的初始头像（首字 / 图标）
--   - 非 NULL = data URL 形式（"data:image/png;base64,xxxxx"），前端直接渲染
--
-- 设计取舍：
--   - 使用 TEXT 而非 BLOB：方便 sqlx 直接 bind & 读；base64 串通常 < 50KB
--     （即使 500x500 PNG 也 < ~330KB）。若后续用户上传大头像超过 SQLite
--     单行字段（1GB）后可考虑迁到独立文件表。
--   - 不创建索引（avatar 不参与查询）
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 不影响已有数据（默认 NULL = 沿用现有头像）
-- ============================================================================

ALTER TABLE agents ADD COLUMN avatar TEXT;
