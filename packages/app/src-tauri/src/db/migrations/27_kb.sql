-- =========================================================================
-- IcePaw 数据库迁移 V27：知识库（RAG v1，agentic 检索）
-- 三级归属：agent 专业 / project（预留） / global 兜底
-- =========================================================================

-- 知识库（KB）配置：每个 KB 监听一个目录，scope 决定归属层级
CREATE TABLE IF NOT EXISTS kb (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    scope      TEXT NOT NULL,                 -- 'agent' | 'project' | 'global'
    owner_id   TEXT,                          -- agent_id / project_id / NULL(global)
    directory  TEXT NOT NULL,                 -- 监听的知识库目录绝对路径
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_kb_scope_owner ON kb(scope, owner_id);

-- 知识库文档索引：v1 文档级（无向量/切块），关键词检索
CREATE TABLE IF NOT EXISTS kb_document (
    id           TEXT PRIMARY KEY,
    kb_id        TEXT NOT NULL REFERENCES kb(id) ON DELETE CASCADE,
    file_path    TEXT NOT NULL,               -- 相对 kb.directory 的路径
    title        TEXT NOT NULL DEFAULT '',
    summary      TEXT NOT NULL DEFAULT '',
    tags         TEXT NOT NULL DEFAULT '[]',  -- JSON 数组（frontmatter 提取）
    content_hash TEXT,                        -- 变更检测（增量索引用）
    file_mtime   TEXT,                        -- 源文件修改时间
    indexed_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_kb_document_kb ON kb_document(kb_id);
