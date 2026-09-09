-- MA-3 跨会话通讯：会话级收件政策。
--
-- 三态（CC 实测经验进不变式，2026-09-09 调研）：
--   'hold'   —— 来件扣住待用户批准（默认，保守起步：agent 间投递自动触发
--               LLM 回合是花钱+扰动行为，默认不该静默发生）
--   'accept' —— 来件排队、目标空闲时自动消费（用户显式信任的协作对手）
--   'refuse' —— 拒收（投递方工具立即报错，源 agent 可感知）
--
-- 会话级粒度（非 agent 级）：@ 提及与收件单位都是会话，同一 agent 的多个
-- 会话可有不同政策。pending 队列不存表——由 session_events 的
-- cross_session_message（投递）+ cross_session_message_settled（终态）对
-- 定义（find_open_turns 同款「有 A 无 B」查询），append-only 不变式保持。

ALTER TABLE conversations ADD COLUMN inbox_policy TEXT NOT NULL DEFAULT 'hold';
