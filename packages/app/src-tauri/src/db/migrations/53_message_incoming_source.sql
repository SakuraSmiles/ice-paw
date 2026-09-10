-- MA-3 来件元数据（2026-09-10 修复批）：消费回合物化的 user 消息带来源
-- 标注。旧形态 = content 文本前缀（`[来自会话「…」]`，前端解析文本渲染
-- incoming 卡）——脆弱且把「系统规范」降级成「文本约定」。
--
-- 新形态：本列存投递方身份快照 JSON（IncomingSourceMeta：
--   {"source_conversation_id","source_conversation_title","source_agent_name"}），
-- 系统组装（inbox 引擎），前端 incoming 卡的权威数据源；文本前缀仅作
-- LLM 视角投影与 legacy 兜底（旧行 NULL → 前端回落 parseIncomingText）。
--
-- 事件侧对称：user_message 事件 payload 加同名字段（serde default 零迁移），
-- derive 透传保持「派生视图 == legacy 行视图」不变式。

ALTER TABLE messages ADD COLUMN incoming_source TEXT;
