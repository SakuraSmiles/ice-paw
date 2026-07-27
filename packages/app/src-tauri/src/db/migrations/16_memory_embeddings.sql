-- =========================================================================
-- IcePaw Migration 16: memory_embeddings 表（REQ-CHAT-047 语义检索）
-- ----------------------------------------------------------------------------
-- 用途：
--   存储每个 agent 的「记忆文本」及其对应的 embedding 向量，
--   用于基于 cosine 相似度的语义检索。
--
-- 设计要点：
--   - id 由调用方生成（uuid）
--   - agent_id 标识「属于哪个 agent」的记忆（同一向量空间）
--   - content 为原始文本（检索命中时返回原文）
--   - embedding 为 BLOB（小端序 f32 数组，dim 通常 1536）
--   - created_at 默认当前时间，便于后续按时间衰减
--
-- 索引：
--   idx_memory_embeddings_agent —— 按 agent_id 过滤时走索引扫描
--   （注意：向量维度通常 1536，BLOB 可能很大，
--    无需对 embedding 建索引 —— 相似度计算在内存里完成）
-- =========================================================================

CREATE TABLE memory_embeddings (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    content TEXT NOT NULL,
    -- embedding: BLOB，little-endian f32 数组（4 字节 / 维度）
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 按 agent_id 过滤（recall 时每次都按 agent_id 过滤）
CREATE INDEX idx_memory_embeddings_agent
    ON memory_embeddings(agent_id);