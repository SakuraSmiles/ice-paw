-- =========================================================================
-- IcePaw 数据库迁移 V30：知识库文档切块 + 向量（RAG v2 语义检索）
-- =========================================================================

-- 文档切块：把文档按段落切成 chunk，每个 chunk 生成 embedding 向量
CREATE TABLE IF NOT EXISTS kb_document_chunk (
    id          TEXT PRIMARY KEY,
    doc_id      TEXT NOT NULL REFERENCES kb_document(id) ON DELETE CASCADE,
    chunk_idx   INTEGER NOT NULL,             -- 第几块（0-based）
    content     TEXT NOT NULL,                -- chunk 原文
    embedding   BLOB,                         -- 向量（f32 数组序列化），NULL=未生成
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_kb_chunk_doc ON kb_document_chunk(doc_id);
