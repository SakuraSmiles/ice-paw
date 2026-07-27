-- ============================================================================
-- IcePaw Migration 21: 项目主题色
-- ----------------------------------------------------------------------------
-- 背景：REQ-PROJ-001b 在 ProjectFormModal 中提供主题色选择器（10 色调色板），
--       用户可主动选择颜色覆盖默认的「按名称哈希」派生结果。
--
-- 设计：
--   - 新增 theme_color TEXT 列（NULL = 未选择，使用 accentFromName 派生）
--   - 旧项目自动 theme_color IS NULL，渲染层 fallback 到 hash 派生
--   - 允许的取值范围由前端把控（参见 src/utils/projectAccent.ts），
--     后端做白名单校验（只接受 ProjectAccent 之一）
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 引入新列（NULL 默认值），老项目自动 fallback
--   - sqlx::migrate! 按文件名执行且内置迁移表，不会重复执行
-- ============================================================================

ALTER TABLE projects ADD COLUMN theme_color TEXT;
