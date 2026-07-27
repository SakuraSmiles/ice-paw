-- =========================================================================
-- IcePaw Migration 19: agents.description（REQ-AGENT-001）
-- ----------------------------------------------------------------------------
-- 用途：
--   给 agents 表增加 description 字段（用户对 Agent 的简短描述）。
--   - 0~500 字符（与 CSCI v4 阶段 1 validate.rs 中 MAX_DESCRIPTION_LEN 对齐）
--   - ''（空串）= 未填写
--   - NULL = 不适用（暂未使用，等价于 ''）
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 不影响已有数据（默认 ''）
--   - 不创建索引（description 不参与高频查询）
-- ============================================================================

ALTER TABLE agents ADD COLUMN description TEXT NOT NULL DEFAULT '';
