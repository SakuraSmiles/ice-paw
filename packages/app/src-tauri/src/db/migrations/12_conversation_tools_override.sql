-- Task 3b: 对话级工具覆盖
-- JSON 格式：{"read_file": true, "list_directory": false}
-- NULL = 继承 Agent 配置，不覆盖
ALTER TABLE conversations ADD COLUMN tools_override TEXT DEFAULT NULL;
