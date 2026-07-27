-- =========================================================================
-- IcePaw 数据库迁移 V22：Agent ↔ Template 关联（REQ-TMPL-002d）
--
-- 来源：CSCI v4 阶段 8.2
-- 说明：
--   - CSCI 规格 REQ-TMPL-002d 要求删除模板前检查是否被 Agent 引用。
--   - 现有 schema 没有 agent ↔ template 的关联关系，因此本迁移先建立
--     多对多关联表 `agent_templates`（与 `project_agents` 同模式）。
--   - 关联列上的双向 CASCADE：删除 Agent 清理其关联、删除 Template 清理
--     其关联，这样 `delete_template` 的引用检查只需看 `agent_templates`
--     是否有目标 template_id 的记录。
--   - 索引聚焦 `template_id`（删除前的引用计数查询热点），反向
--     `agent_id` 索引待实际跨表查询出现后再补，避免冷索引。
-- =========================================================================

CREATE TABLE IF NOT EXISTS agent_templates (
    agent_id    TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    template_id TEXT NOT NULL REFERENCES templates(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (agent_id, template_id)
);

-- 用于 REQ-TMPL-002d 删除模板前的引用计数：
--   SELECT COUNT(*) FROM agent_templates WHERE template_id = ?
-- 走 `idx_agent_templates_template` 时只触按 template_id 索引，免全表扫描。
CREATE INDEX IF NOT EXISTS idx_agent_templates_template
    ON agent_templates(template_id);
