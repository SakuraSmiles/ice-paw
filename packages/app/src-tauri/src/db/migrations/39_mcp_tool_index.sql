-- ============================================================================
-- IcePaw Migration 39: MCP Server tool_index（OpenAI 合规工具命名空间索引）
-- ----------------------------------------------------------------------------
-- 背景：MCP 工具名原由「中文 server 显示名 + 点号 + 工具名」拼成
--       （如 `浏览器自动化.browser_click`），而 OpenAI 兼容端点（deepseek /
--       minimax / glm 等）要求 function name 严格匹配 `^[a-zA-Z0-9_-]+$`
--       —— 中文字符和点号都违规，触发 HTTP 400。
--       改为 `t{tool_index}_{raw_tool_name}`：整数前缀必然合规、且跨重启稳定
--       （见 harness/mcp/manager.rs 的 namespaced_tool_name）。历史里已持久化的
--       违规名字由 context/history.rs 加载期就地 sanitize，无需数据迁移。
--
-- 设计：
--   - 新增 tool_index INTEGER NOT NULL DEFAULT 0
--       每个 server 一个稳定整数命名空间索引；create() 用 (MAX+1) 原子递增分配。
--   - 回填：builtin 三件套固定 0/1/2（与 seed 顺序一致）；其余用户自建 server
--       按 created_at 接续，保证索引互不冲突。
--
-- 兼容性：
--   - 全新安装：本 migration 跑时 mcp_servers 表尚空，UPDATE 命中 0 行；
--     随后 seed_defaults 按 create() 的 MAX+1 自动得到 0/1/2，两路径收敛（同 35 模式）。
--   - sqlx::migrate! 在 db init 执行（早于 seed_defaults），按文件名执行不重复。
--   - ROW_NUMBER() 窗口函数 + UPDATE FROM 需 SQLite ≥3.33/3.25，sqlx 自带
--     libsqlite3-sys（≥3.40）满足。
-- ============================================================================

ALTER TABLE mcp_servers ADD COLUMN tool_index INTEGER NOT NULL DEFAULT 0;

UPDATE mcp_servers
SET tool_index = t.new_idx
FROM (
    SELECT id,
           ROW_NUMBER() OVER (
               ORDER BY CASE id WHEN 'builtin-thinking'   THEN 0
                                WHEN 'builtin-memory'     THEN 1
                                WHEN 'builtin-playwright' THEN 2
                                ELSE 100
               END,
                        created_at
           ) - 1 AS new_idx
    FROM mcp_servers
) AS t
WHERE mcp_servers.id = t.id;
