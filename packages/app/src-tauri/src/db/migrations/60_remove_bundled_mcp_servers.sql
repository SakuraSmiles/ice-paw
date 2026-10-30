-- ============================================================================
-- IcePaw Migration 60: 移除 bundled MCP server 种子行（mcp-runtime 原生化）
-- ----------------------------------------------------------------------------
-- 背景：builtin-thinking（sequential-thinking）与 builtin-memory（知识图谱）
--       原走内置 Node 运行时（node.exe + 预打包 node_modules，~124MB）。现转为
--       Rust 原生内置工具（harness::mcp::sequential_thinking / memory_graph），
--       server 行不再需要，删掉以释放安装包体积并统一「内置工具 vs 外部 server」
--       边界（内置工具走 bare 名、不经 server 命名空间 t{idx}_）。
--
-- 兼容性：
--   - 只删这两行；builtin-filesystem 早已于 v0.2.5 下线，此处不含。
--   - 用户若曾手动新建/改名过这些 id，DELETE 按 id 精确匹配，不误伤其他行。
--   - 全新安装：本 migration 跑时表尚空，DELETE 命中 0 行；随后 seed_defaults
--     也只播种 builtin-playwright（bundled 播种已从 default_mcp_servers 移除），
--     两路径收敛。
--   - runtime_kind 列保留（migration 35 已发布不可改），历史 'bundled' 值经
--     FromStr 回落 System（见 harness::mcp::types::RuntimeKind）。
-- ============================================================================

DELETE FROM mcp_servers WHERE id = 'builtin-thinking';
DELETE FROM mcp_servers WHERE id = 'builtin-memory';
