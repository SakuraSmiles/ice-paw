-- ModelProfile Phase 2 批 1：agent 模型引用 + 降级链占位
--
-- model_profile_id：引用 model_profiles.id（无 FK——profile 删除守卫在命令层三段式拦截，
-- 与 vision/embedding 引用守卫同款；悬空引用读侧降级 legacy 快照列继续可用）。
-- NULL = legacy 路径（provider/model/api_key_ref/base_url 四列即权威），存量 agent 零迁移。
-- 非 NULL = 四列作解析快照（get_with_credentials 每轮解析回写，值变才写）。
--
-- fallback_profile_ids：降级链（批 2 消费）。JSON 数组串，如 '["mp-1","mp-2"]'
-- （enabled_tools 列同款惯例）。NULL/空 = 无降级链，行为与现状逐字节一致。

ALTER TABLE agents ADD COLUMN model_profile_id TEXT;
ALTER TABLE agents ADD COLUMN fallback_profile_ids TEXT;
