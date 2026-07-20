-- Phase 2: 项目维度管理
-- 默认项目 = conversations.project_id IS NULL（不在此表创建记录）

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    icon        TEXT NOT NULL DEFAULT 'folder',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 项目 ↔ Agent 多对多关联
CREATE TABLE IF NOT EXISTS project_agents (
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'member',  -- 'lead' | 'member'
    joined_at   TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (project_id, agent_id)
);

-- 会话加 project_id（nullable = 默认项目）
-- ALTER TABLE ... ADD COLUMN 是幂等的：若列已存在会报错，但 sqlx migrate 不会重复执行。
ALTER TABLE conversations ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL;

-- 部分索引：仅对非 NULL 的 project_id 建索引，默认项目（NULL）不占索引空间
CREATE INDEX IF NOT EXISTS idx_conversations_project
    ON conversations(project_id) WHERE project_id IS NOT NULL;
