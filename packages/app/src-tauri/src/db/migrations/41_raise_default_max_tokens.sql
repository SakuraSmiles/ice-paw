-- 抬升历史 agent 的输出 token 上限：4096 → 16384
-- 旧的 4096 默认值会截断稍长的回答（finish_reason=length / max_tokens）。
-- 此前 agent 表单无此字段，所有 4096 均为系统默认（非用户显式设置），可安全批量抬升。
-- 用户显式设置过其它值（如 agent.yaml 里 8192/16384）的行不受影响。
UPDATE agents SET max_tokens = 16384 WHERE max_tokens = 4096;
