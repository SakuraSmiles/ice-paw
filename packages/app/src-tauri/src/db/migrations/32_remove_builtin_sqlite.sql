-- 32_remove_builtin_sqlite.sql
-- 删除已弃用的 builtin-sqlite 默认 MCP server：
-- @modelcontextprotocol/server-sqlite 包 npm 404（启动失败），且查询的
-- {workspace}/data.db 多数用户不存在（不实用）。default_mcp_servers() 已同步移除。
DELETE FROM mcp_servers WHERE id = 'builtin-sqlite';
