-- Migration 48: projects.avatar — 项目头像图片（base64 dataURL，前端压缩 ≤256px）
-- 可空零回填；不选走前端名字哈希渐变兜底（EntityAvatar 三级链）。
-- emoji 沿用既有 icon 列（不再单列）；主题色沿用 theme_color。
ALTER TABLE projects ADD COLUMN avatar TEXT;
