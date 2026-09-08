-- Migration 50: model_profiles 健康三列 + 触发器 WHEN 门控（状态监控，2026-09-08）
--
-- 健康三列：last_health = 按最后一次真实调用结果分类的 slug（ok/quota/auth/
-- rate_limited/network/model_not_found/unknown，NULL = 从未调用）；
-- last_health_detail = 失败原文（截断 300 字符，hover 诊断用）；
-- last_health_at = 最后一次调用时间。全 NULL = 未调用（前端「未调用」态，
-- 绝不冒充「正常」）。分类单一真相源 harness/profile_health.rs。
--
-- 触发器换 WHEN 门控版：健康记录（record_health）每轮真实调用都会发生，
-- 49 版触发器任何 UPDATE 都刷 updated_at → 健康记录伪造成「编辑过」。
-- WHEN 用 IS NOT 做 NULL 安全比较，纯健康列 UPDATE 不触发；repo::update
-- 同值 SET 也不触发。
ALTER TABLE model_profiles ADD COLUMN last_health TEXT;
ALTER TABLE model_profiles ADD COLUMN last_health_detail TEXT;
ALTER TABLE model_profiles ADD COLUMN last_health_at TEXT;

DROP TRIGGER trg_model_profiles_upd;
CREATE TRIGGER trg_model_profiles_upd
  AFTER UPDATE ON model_profiles
  WHEN NEW.alias IS NOT OLD.alias OR NEW.provider IS NOT OLD.provider
    OR NEW.model IS NOT OLD.model OR NEW.base_url IS NOT OLD.base_url
    OR NEW.sort_order IS NOT OLD.sort_order
  BEGIN
    UPDATE model_profiles SET updated_at = datetime('now') WHERE id = NEW.id;
  END;
