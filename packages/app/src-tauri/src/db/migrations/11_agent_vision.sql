-- Task: supports_vision 字段
-- Agent 是否支持图片输入（控制 ChatInput 📎 按钮是否可用）
ALTER TABLE agents ADD COLUMN supports_vision INTEGER DEFAULT 0;
