-- Agent 的托管配置归属与工具工作区解耦。
-- 文件迁移由 db::agent_config 在 SQL migrations 后执行；成功才登记目录。
ALTER TABLE agents ADD COLUMN config_dir TEXT;
