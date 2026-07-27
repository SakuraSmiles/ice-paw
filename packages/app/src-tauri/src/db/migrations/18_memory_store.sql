-- =========================================================================
-- IcePaw Migration 18: memory_store 表（REQ-CHAT-048 记忆加密存储）
-- ----------------------------------------------------------------------------
-- 用途：
--   存储每个 agent 的「记忆」（典型：对话摘要 / 长期事实），
--   内容使用 XChaCha20-Poly1305 加密后以 BLOB 形式持久化。
--
-- 设计要点：
--   - id 由调用方生成（uuid）
--   - agent_id 标识「属于哪个 agent」的记忆（同一加密命名空间）
--   - content_encrypted 是密文 BLOB（格式：[nonce:24][ct:N][tag:16]）
--     由 `crate::crypto::encrypt_blob` 加密；解密调 `crate::crypto::decrypt_blob`
--   - content_type 区分记忆类型（'summary' / 'fact' / 'note' 等），
--     默认 'summary' 与现有 M1.5 摘要消息语义对齐
--   - created_at 默认当前时间，便于按时间排序
--
-- 与 memory_embeddings（REQ-CHAT-047）的关系：
--   - memory_embeddings: 语义检索用，向量形式
--   - memory_store:     记忆原文（加密），用于 LLM 注入 / 审计 / 恢复
--   两表同一 agent 下并行存在，互不干扰
--
-- 索引：
--   idx_memory_store_agent       —— 按 agent_id 过滤
--   idx_memory_store_agent_type  —— 按 (agent_id, content_type) 复合过滤
--     （典型查询："取该 agent 的最新摘要" = WHERE agent_id=? AND content_type='summary'）
-- =========================================================================

CREATE TABLE memory_store (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    -- 密文 BLOB：XChaCha20-Poly1305(nonce 24 + ciphertext N + tag 16)
    content_encrypted BLOB NOT NULL,
    -- 记忆类型：默认 'summary'（M1.5 摘要消息语义）；
    -- 未来扩展 'fact' / 'note' / 'tool-result' 等
    content_type TEXT NOT NULL DEFAULT 'summary',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 按 agent_id 过滤
CREATE INDEX idx_memory_store_agent
    ON memory_store(agent_id);

-- 按 (agent_id, content_type) 复合过滤（取该 agent 最新摘要时走该索引）
CREATE INDEX idx_memory_store_agent_type
    ON memory_store(agent_id, content_type);
