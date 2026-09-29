-- 58_conversation_auto_approve.sql — 会话级全自动开关（2026-09-29 用户拍板）
-- 开启后本会话（含其发起的委派子会话——delegate 创建时继承标志）的 Confirm 级
-- 工具调用直接放行，不再弹审批卡。例外：屏幕家族不沾光（走屏幕共享通道的
-- 独立授权体系 + Off 提议制，见 screen/channel.rs）；「禁用的 MCP server 永不
-- 自动复活」等治理动作不受影响（那是 server 状态不是工具授权）。

ALTER TABLE conversations ADD COLUMN auto_approve INTEGER NOT NULL DEFAULT 0;
