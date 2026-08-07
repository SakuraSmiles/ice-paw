-- ============================================================================
-- IcePaw Migration 35: MCP Server runtime_kind（内置运行时标记）
-- ----------------------------------------------------------------------------
-- 背景：内置默认 server（filesystem / sequential-thinking / memory）此前用
--       `npx -y <pkg>` 运行时拉取，在 GFW 环境 / npm 缓存损坏时极易失败
--       （生产曾出现 sequential-thinking 缺传递依赖 zod 而启动失败）。
--       改为「内置 Node 运行时 + 预打包 node_modules」，安装包自带、零网络依赖。
--
-- 设计：
--   - 新增 runtime_kind TEXT NOT NULL DEFAULT 'system'
--       'system'  → command 走系统 PATH（npx/node/pipx，依赖系统 node）——默认，用户自加 server 用此
--       'bundled' → 用 IcePaw 内置 node.exe + 打包好的 node_modules（零网络依赖）
--   - 把已存在的 3 个 builtin 行 UPDATE 为 bundled：
--       command 改 'node'、args 去掉包名（包名/入口由 start_server 解析注入）。
--       只动 runtime_kind / command / args 三列，不碰 name / enabled / trust_level
--       （用户对这些的改动保留）；若用户深度定制过 args，可在 DB 改回 system/npx 还原。
--
-- 兼容性：
--   - ALTER TABLE ADD COLUMN 引入新列（DEFAULT 'system'），老 server / 用户自加 server 自动视为 system。
--   - 全新安装：本 migration 跑时 mcp_servers 表尚空，3 条 UPDATE 命中 0 行；
--     随后 seed_defaults 直接播种 runtime_kind='bundled' 的 3 个 builtin，两路径收敛。
--   - sqlx::migrate! 在 db init 执行（早于 seed_defaults），按文件名执行不重复。
-- ============================================================================

ALTER TABLE mcp_servers ADD COLUMN runtime_kind TEXT NOT NULL DEFAULT 'system';

UPDATE mcp_servers SET runtime_kind = 'bundled', command = 'node', args = '["{workspace}"]'
    WHERE id = 'builtin-filesystem';
UPDATE mcp_servers SET runtime_kind = 'bundled', command = 'node', args = '[]'
    WHERE id = 'builtin-thinking';
UPDATE mcp_servers SET runtime_kind = 'bundled', command = 'node', args = '[]'
    WHERE id = 'builtin-memory';
