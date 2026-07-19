-- =========================================================================
-- IcePaw 数据库迁移 V7：消息摘要关联（M1.5 A3-4 滚动摘要）
-- 来源：dev1 m1-review.md §4.1 + dev2 m1-implementation-design.md M1.5
-- 说明：
--   1) messages 表新增 summary_id 列（TEXT，可空，外键指向 messages.id）
--      - NULL = 该消息未被任何摘要覆盖（默认状态）
--      - 指向某条 role="system" 的摘要消息 ID = 该消息已被该摘要覆盖
--   2) 摘要消息本身是一条 role="system" 的消息，content 以
--      "[Previous conversation summary]" 开头，summary_id = NULL
--   3) 采用 "messages.summary_id" 方案而非新建 message_summaries 表：
--      - 减少 schema 变更面（只加一列，不需要新表 + 新 repo + 新索引）
--      - 查询简单（加载历史时按正常逻辑加载，摘要随消息一起出现）
--      - 多版本自然支持（新摘要 → 插入新 system 消息，旧消息指向新摘要）
-- =========================================================================

ALTER TABLE messages ADD COLUMN summary_id TEXT REFERENCES messages(id);
