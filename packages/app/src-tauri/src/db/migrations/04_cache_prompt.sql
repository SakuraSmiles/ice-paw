-- =========================================================================
-- IcePaw 数据库迁移 V4：Prompt Caching（P2-3）
-- 来源：icepaw-p2-2-3-design.md §2 P2-3
-- 说明：
--   1) agents 表新增 cache_prompt 列（INTEGER 0/1，默认 1）
--      - true（默认）：Anthropic adapter 注入 cache_control 断点（≤4 个）
--      - false：跳过注入，请求体保持纯文本结构
--   2) OpenAI 不需要此字段：OpenAI Chat Completions API 对 ≥1024 token 的
--      前缀自动缓存（zero-config），无需显式标记。
-- =========================================================================

ALTER TABLE agents ADD COLUMN cache_prompt INTEGER NOT NULL DEFAULT 1;