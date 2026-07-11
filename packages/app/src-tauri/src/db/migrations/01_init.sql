-- =========================================================================
-- IcePaw 数据库初始化脚本（V1）
-- 来源：icepaw-cleanup-plan.md §2.2
-- 说明：
--   1) 三张表：agents / conversations / messages
--   2) 外键级联删除：删除 agent 自动清理 conversations，删除 conversation 自动清理 messages
--   3) 触发器自动维护 updated_at
--   4) 索引按查询热点（按 agent_id、按 conversation_id + 时间序）
-- =========================================================================

PRAGMA foreign_keys = ON;

-- ----------------------------- agents -----------------------------------
CREATE TABLE agents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  system_prompt TEXT NOT NULL DEFAULT '',
  api_key_ref TEXT NOT NULL,                       -- 仅存 stronghold 中的引用 key（默认 = agent_id）
  base_url TEXT,                                   -- 可选：自定义 OpenAI 兼容网关
  temperature REAL NOT NULL DEFAULT 0.7,
  max_tokens INTEGER NOT NULL DEFAULT 4096,
  extra_params TEXT NOT NULL DEFAULT '{}',         -- JSON 字符串，前端按需扩展
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ----------------------------- conversations ----------------------------
CREATE TABLE conversations (
  id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  title TEXT NOT NULL DEFAULT '',
  pinned INTEGER NOT NULL DEFAULT 0,               -- 0/1 布尔
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_conversations_agent   ON conversations(agent_id);
CREATE INDEX idx_conversations_updated ON conversations(updated_at DESC);

-- ----------------------------- messages --------------------------------
CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL,                              -- 'system' | 'user' | 'assistant' | 'tool'
  content TEXT NOT NULL,
  token_count INTEGER,                             -- 可选：流式完成后回填
  error TEXT,                                      -- 可选：失败原因
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at);

-- ----------------------------- 触发器：自动更新 updated_at ---------------
CREATE TRIGGER trg_agents_upd
  AFTER UPDATE ON agents
  BEGIN
    UPDATE agents SET updated_at = datetime('now') WHERE id = NEW.id;
  END;

CREATE TRIGGER trg_conversations_upd
  AFTER UPDATE ON conversations
  BEGIN
    UPDATE conversations SET updated_at = datetime('now') WHERE id = NEW.id;
  END;
