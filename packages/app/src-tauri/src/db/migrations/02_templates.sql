-- =========================================================================
-- IcePaw 数据库迁移 V2：用户自定义模板（templates 表）
-- 来源：icepaw-p0-p2-plan.md §2.4 P2-4
-- 说明：
--   - 模板是「带变量占位符的 system prompt + user prompt 前缀」组合
--   - variables 存 JSON 数组：[{name,label,type,default,options?}]
--   - tools 存 JSON 数组：["read_file","shell_command",...]（P2-1 落地后实际生效）
--   - sort_order 列表排序权重（小者靠前）
--   - 时间戳触发器（updated_at 自动维护）
-- =========================================================================

CREATE TABLE templates (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  system_prompt TEXT NOT NULL DEFAULT '',
  user_prompt_prefix TEXT NOT NULL DEFAULT '',
  -- 变量定义（JSON 数组）
  variables TEXT NOT NULL DEFAULT '[]',
  -- 关联的工具名列表（JSON 数组）
  tools TEXT NOT NULL DEFAULT '[]',
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_templates_sort ON templates(sort_order ASC, created_at ASC);

-- 触发器：自动维护 updated_at
CREATE TRIGGER trg_templates_upd
  AFTER UPDATE ON templates
  BEGIN
    UPDATE templates SET updated_at = datetime('now') WHERE id = NEW.id;
  END;
