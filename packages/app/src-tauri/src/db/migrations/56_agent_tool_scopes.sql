-- 工具集权限控制（2026-09-12 拍板）：agents 增加 tool_scopes 列
-- JSON 数组串，条目三形态：group:<内置组键> / server:<MCP server 配置 id> / 裸工具名
-- NULL / 空 = 全部工具（默认全开，与 enabled_tools 的「空 ≡ 全开」同一约定）
-- 与既有 enabled_tools 白名单串联过滤（先 scopes 后名单），存量 agent 零迁移零行为变化
ALTER TABLE agents ADD COLUMN tool_scopes TEXT;
