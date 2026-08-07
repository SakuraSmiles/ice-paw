-- ============================================================================
-- IcePaw Migration 36: 移除内置种子 builtin-filesystem / builtin-maifady
-- ----------------------------------------------------------------------------
-- 背景：
--   - 文件系统工具集（builtin-filesystem）的能力已由 native 内置工具完全覆盖且更优
--     （read / write / edit / delete / move_file / create_directory / directory_tree /
--      get_file_info / read_multiple_files / search_files，见 harness::mcp::internal /
--      file_tools / search），不再需要独立的 MCP Server 进程。
--   - 工程专家团队（builtin-maifady）价值低、依赖系统 node + npx 联网拉取，不再随产品内置。
--
-- 处理：删除已安装用户 DB 里的这两条种子记录。default_mcp_servers() 已同步移除，
--       seed_defaults（仅补种缺失项）不会重新写入，故永久清除。
--
-- 兼容性：
--   - 全新安装：本 migration 命中 0 行（表为空或无此 id），无副作用。
--   - sqlx::migrate! 在 db init 执行（早于 seed_defaults），按文件名顺序执行、不重复。
-- ============================================================================

DELETE FROM mcp_servers WHERE id IN ('builtin-filesystem', 'builtin-maifady');
