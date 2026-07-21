-- ============================================================================
-- IcePaw Migration 14: 项目空间路径
-- ----------------------------------------------------------------------------
-- 背景：项目管理需要记录每个项目对应的本地工作区根目录（代码根、文档根、
--       多子文件夹的父目录等），方便 Agent 在工具调用（read_file / list_dir）
--       时知道操作的根。路径由用户在前端填写，存原文（不规范化），后端校验
--       是否存在并提示。
--
-- 兼容性：
--   - 使用 ALTER TABLE ADD COLUMN 引入新列（NULL 默认值），老项目自动 workspace_path IS NULL
--   - sqlx::migrate! 按文件名执行且内置迁移表，不会重复执行
--   - 不修改 project_agents 表（成员编辑走 DELETE + 批量 INSERT）
-- ============================================================================

ALTER TABLE projects ADD COLUMN workspace_path TEXT;

-- 部分索引：只索引非 NULL 的项目，节省空间。
-- 应用场景：未来若需要「列出所有有工作区的项目」可走索引扫描。
CREATE INDEX IF NOT EXISTS idx_projects_workspace_path
    ON projects(workspace_path) WHERE workspace_path IS NOT NULL;
