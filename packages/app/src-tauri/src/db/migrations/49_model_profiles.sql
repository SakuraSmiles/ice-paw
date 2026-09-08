-- Migration 49: model_profiles 表 —— 模型配置实体化（ModelProfile）Phase 1
-- 一条 = provider + model + key 引用 + base_url + 别名（用户可读，如「智谱主力」「智谱 Coding」）。
-- key 密文存 Stronghold（槽位 profile:{id}，同 agents 的 api_key_ref 惯例），表内只存引用。
-- 视觉读取 / 语义检索改为引用本表（preferences 新键 vision_profile_ids / embedding_profile_id）；
-- agent 链路零接触（Phase 2 另立）。刻意不设 (provider, model) 与 alias 唯一约束——
-- 「智谱主力 vs 智谱 Coding」同模型名不同端点、「主 key vs 备用 key」全同字段仅 key 不同，
-- 唯一性由别名区分。
--
-- ⚠️ 已应用后不可变（2026-09-08 实案）：本文件在开发期曾被改写（加健康三列），
-- 而 dev 真机库已应用原版 → SELECT 报 no such column。已还原为原 schema，
-- 健康三列拆进 migration 50；checksum 字节差由 heal_checksum_drift 自愈。
CREATE TABLE model_profiles (
  id TEXT PRIMARY KEY,
  alias TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  api_key_ref TEXT NOT NULL,
  base_url TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_model_profiles_sort ON model_profiles(sort_order ASC, created_at ASC);

CREATE TRIGGER trg_model_profiles_upd
  AFTER UPDATE ON model_profiles
  BEGIN
    UPDATE model_profiles SET updated_at = datetime('now') WHERE id = NEW.id;
  END;
