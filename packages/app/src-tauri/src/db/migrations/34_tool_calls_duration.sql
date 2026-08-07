-- =========================================================================
-- IcePaw 数据库迁移 V34：tool_calls 审计表补充耗时列
-- 来源：补全 tool_calls 审计（03 建表后从未写入，现已由 tool_executor 接入）
-- 说明：
--   tool_executor 已计算每次工具调用的 duration_ms。落到 tool_calls 表后，
--   可直接 ORDER BY duration_ms 排查慢命令，无需从 finished_at - started_at 推算。
-- =========================================================================

ALTER TABLE tool_calls ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;
