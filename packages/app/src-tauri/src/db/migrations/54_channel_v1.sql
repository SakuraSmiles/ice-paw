-- 频道 v1（0.7 批 C）：项目 1:1 串行共享流会话
-- 设计真相源：docs/channel-v1-design.md §2

-- 发送者归属：共享流多成员发言的行级归因（NULL = 用户消息或 1v1 隐含会话 agent）
ALTER TABLE messages ADD COLUMN sender_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL;

-- 频道归档态：项目永久删除时频道软删除为只读归档，聊天记录保留（2026-09-10 拍板）
ALTER TABLE conversations ADD COLUMN archived_at TEXT;

-- 项目 ↔ 频道 1:1 唯一性（部分唯一索引；ensure_channel 幂等的 DB 兜底）
-- 归档频道 project_id 被 FK SET NULL 清空后，NULL 不参与唯一判重——同项目可建新频道
CREATE UNIQUE INDEX idx_channel_per_project ON conversations(project_id) WHERE kind='channel';
