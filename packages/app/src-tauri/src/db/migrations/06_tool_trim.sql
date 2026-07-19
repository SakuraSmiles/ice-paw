-- =========================================================================
-- IcePaw 数据库迁移 V6：Agent 工具裁剪阈值（M1.2 A2-4）
-- 说明：
--   1) agents 表新增 tool_trim_threshold 列（INTEGER，可空）
--      - NULL = 使用系统默认阈值（ContextBudget.tool_trim_threshold，默认 5）
--      - N>0  = 当注册工具数 >= N 时启用软裁剪（deprioritized 标记）
--      - 旧 Agent 行为完全兼容（NULL = 默认 5）
-- =========================================================================

ALTER TABLE agents ADD COLUMN tool_trim_threshold INTEGER;
