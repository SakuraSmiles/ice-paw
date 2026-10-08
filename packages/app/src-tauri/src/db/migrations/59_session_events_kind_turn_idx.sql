-- =========================================================================
-- IcePaw 数据库迁移 V59：session_events (kind, session_id, turn_id) 复合索引
-- 来源：2026-10-08 生产实案「steer 插话后切页卡死」（生产日志铁证：
-- list_unconsumed_user_anchors 单条 5~25s × 每 3s 一条 → 打满 5 连接池）。
--
-- 根因：session_events 仅有 (session_id, seq) 唯一索引——锚点查询的
-- per-row EXISTS（session_id=? AND turn_id=? AND kind=?）无法索引定位，
-- 大事件量会话（万级）× 每消息行双 EXISTS = 秒级起步。
--
-- 列序 (kind, session_id, turn_id)：
--   - EXISTS 点查（kind 等值 → session/turn 定位）
--   - boot 扫尾的 kind='user_message' 全库扫描（前缀等值）
--   一索引两用。kind 词表可扩展，索引对新 kind 自动生效。
-- =========================================================================

CREATE INDEX IF NOT EXISTS idx_session_events_kind_turn
    ON session_events(kind, session_id, turn_id);
