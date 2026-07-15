-- =========================================================================
-- IcePaw 数据库迁移 V5：每 Agent 历史窗口（A3-2）
-- 来源：icepaw-p0-p2-plan.md §3 A3-2
-- 说明：
--   1) agents 表新增 max_history_messages 列（INTEGER，可空）
--      - NULL  = 使用系统默认值（见 context::history::DEFAULT_HISTORY_WINDOW）
--      - N>0   = 加载最近 N 条历史消息注入 LLM 上下文
--      - 旧 Agent 行为完全兼容（NULL = 默认 20）
--   2) 不同 Agent 可以拥有不同上下文窗口：例如 OpenAI 8K 用 16 条
--      小消息，Anthropic 200K 用 60 条带图片的多模态消息
-- =========================================================================

ALTER TABLE agents ADD COLUMN max_history_messages INTEGER;