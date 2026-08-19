-- ============================================================================
-- IcePaw Migration 47: Agent emoji 头像
-- ----------------------------------------------------------------------------
-- 背景：agent 视觉身份双通道（图片 avatar 列 migration 20 已有 + emoji 本列）。
--       前端 EntityAvatar 三级兜底：image → emoji → 名字哈希渐变+首字。
--
-- 设计：
--   - 新增 emoji TEXT 列（NULL = 未选择，渲染层走渐变兜底）
--   - avatar 复用既有列（base64 dataURL，前端 canvas 压缩 ≤300KB）
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 引入新列（NULL 默认值），旧 agent 自动兜底
-- ============================================================================

ALTER TABLE agents ADD COLUMN emoji TEXT;
