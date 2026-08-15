-- =========================================================================
-- IcePaw 数据库迁移 V45：会话类型 + 委派图边（多 agent 协作 MA-1）
-- 来源：docs/multi-agent-architecture.md §4.1（2026-08-15 评审定稿）
-- 说明：
--   统一 Session 三类型：kind = 'chat'（用户↔agent，全部存量会话默认值）
--   / 'delegation'（agent→agent 委派子会话，MA-1）/ 'channel'（持久通道，MA-3）。
--   所有类型共用 messages / session_events / 轨迹 / 预算 / hooks / read_route，
--   会话类型只影响渲染形态与工具面（delegation 子会话不注册 delegate_to_agent，
--   委派深度=1 是结构性护栏，无需深度计数器）。
--
--   委派图边：parent_conversation_id 指向发起委派的父会话。ON DELETE SET NULL
--   ——父会话删除只断边不删子（审计保留，同 session_events.message_id 无 FK 先例；
--   子会话活得比父久）。
--
--   发起者：initiator_agent_id = 发起委派的 agent（NULL ≡ 用户发起）。
--   不设 FK 到 agents——agent 可删，会话须活得比 agent 久。
--
--   设计变更（相对设计稿 §4.1）：**不建 project_members 表**——项目成员已有
--   project_agents 表（migration 13）+ 完整 CRUD/管理 UI，可调度集合直接复用：
--   conv.project_id 存在且 project_agents 非空 → 成员集合；否则 → 全部 agent。
--
-- 原则：只加可空列/带默认值列，存量读路径零迁移。
-- =========================================================================

ALTER TABLE conversations ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE conversations ADD COLUMN initiator_type TEXT;
ALTER TABLE conversations ADD COLUMN initiator_agent_id TEXT;
ALTER TABLE conversations ADD COLUMN parent_conversation_id TEXT
    REFERENCES conversations(id) ON DELETE SET NULL;

-- 侧栏（kind='chat' 过滤）与项目任务列表（kind='delegation' 按 project 聚合）共用
CREATE INDEX idx_conversations_kind_project ON conversations(kind, project_id);
