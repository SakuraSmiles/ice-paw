-- MA-3 跨会话通讯：会话级收件政策。
--
-- 三态（2026-09-10 测试反馈后拍板：默认 accept + 项目边界）：
--   'accept' —— 来件排队、目标空闲时自动消费（默认：单用户应用所有会话同属
--               一人；投递本身受「同项目」硬边界约束，防 agent 互扰的护栏
--               （队列上限 + 自动消费配额）在引擎层兜底）
--   'hold'   —— 来件扣住待用户批准（显式治理档：用户想逐件过目时切换）
--   'refuse' —— 拒收（投递方工具立即报错，源 agent 可感知）
--
-- 会话级粒度（非 agent 级）：@ 提及与收件单位都是会话，同一 agent 的多个
-- 会话可有不同政策。pending 队列不存表——由 session_events 的
-- cross_session_message（投递）+ cross_session_message_settled（终态）对
-- 定义（find_open_turns 同款「有 A 无 B」查询），append-only 不变式保持。

ALTER TABLE conversations ADD COLUMN inbox_policy TEXT NOT NULL DEFAULT 'accept';
