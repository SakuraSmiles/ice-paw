-- 频道 Router 试点（Router 化 Phase 3 批 P3-1）：频道行级 router 模式开关。
-- 设计真相源：docs/channel-router-roadmap.md §7 Phase 3。
-- 默认 0（旧群聊路径零变化——试点开关按频道打开，新代码路径与旧路径互斥）。
ALTER TABLE conversations ADD COLUMN router_mode INTEGER NOT NULL DEFAULT 0;

-- router 决策回合的宿主会话 kind（isUserChat 正向白名单天然不进侧栏；
-- 注册闸不含它——M4 方案 c：决策/执行分离，router 零工具面；台账不含它）。
-- 无需 schema 变更：conversations.kind 是自由 TEXT（migration 45）。
