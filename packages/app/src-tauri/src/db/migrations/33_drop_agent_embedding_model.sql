-- 33_drop_agent_embedding_model.sql
-- 删除 agents.embedding_model 列：RAG 语义检索统一走全局 user_preferences.embedding_*
-- 配置（provider/model/api_key/base_url，见 resolve_embedding_config），agent 级
-- embedding_model 字段闲置不用，移除以避免误解。
ALTER TABLE agents DROP COLUMN embedding_model;
