-- 频道 Router 化 Phase 1（临时 Worker 原语）：agent 生命周期标记 + router 关联表
-- 设计真相源：docs/channel-router-roadmap.md §6
-- 2026-11-12 批 P1-1：列与表先落、行为面后续批次接线（channel_routers Phase 3 才用）

-- 生命周期标记：standard=用户可见的常规 agent（存量默认，零迁移）/
-- router=项目调度中枢（系统实体，Phase 3 接线）/ ephemeral=临时 worker（批 P1-2 起产生，
-- 用户配置面不可见——过滤收口见前端 agent store visibleAgents 与 resolve_dispatchable）
ALTER TABLE agents ADD COLUMN agent_kind TEXT NOT NULL DEFAULT 'standard';

-- 项目 ↔ router 1:1（一项目一 router；频道懒建语义不变，router 实体先于频道存在）
-- 本批仅建表不接线；ON DELETE CASCADE = 项目删除连带治理配置（agent 行本身由
-- agents 表生命周期管理，不随项目级联）
CREATE TABLE channel_routers (
  project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  agent_id TEXT NOT NULL REFERENCES agents(id),
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
