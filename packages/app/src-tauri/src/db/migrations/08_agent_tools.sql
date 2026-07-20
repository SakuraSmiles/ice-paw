-- Task 4: Agent 级工具权限配置
-- NULL = 全部启用（向后兼容现有 Agent）
-- 非 NULL = 仅启用数组中列出的工具
ALTER TABLE agents ADD COLUMN enabled_tools TEXT DEFAULT NULL;
