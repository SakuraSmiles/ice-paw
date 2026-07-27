-- =========================================================================
-- IcePaw Migration 17: agents.embedding_model（REQ-CHAT-047 语义检索）
-- ----------------------------------------------------------------------------
-- 用途：
--   给每个 agent 增加可选的 embedding_model 字段。
--   - NULL = 后端默认使用 text-embedding-3-small
--   - 非 NULL = 用户显式指定（如 text-embedding-3-large、自部署 bge-large 等）
--
-- 兼容性：
--   - 使用 ALTER TABLE ADD COLUMN 引入新列（NULL 默认值），老数据自动 NULL
--   - 不影响现有索引（无新索引需求）
-- ============================================================================

ALTER TABLE agents ADD COLUMN embedding_model TEXT;