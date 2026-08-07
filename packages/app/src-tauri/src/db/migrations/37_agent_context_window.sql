-- =========================================================================
-- 37: Agent 上下文窗口（Phase 0 · 上下文预算地基）
-- 说明：
--   1) agents 表新增 context_window 列（INTEGER，可空）
--      - NULL   = 调用时按 (provider, model) 查已知模型默认表，
--                 命中则用默认（如 MiniMax-M3=1M），否则回退 128K
--      - N>0    = 该 agent 显式覆盖（自定义/本地模型、或想保守限制时用）
--   2) 不回填存量数据：运行时解析（agent.context_window → 已知默认 → 128K）
--      统一覆盖存量与新 agent，无需 UPDATE。
--   3) 用途：注入 ContextBudget.max_input_tokens（Phase 1 的 token 窗口
--      stage 将据此动态截断；Phase 0 先让真实值到位）。
--   - sqlx::migrate! 在 db init 执行（早于 seed_defaults），按文件名执行不重复。
-- =========================================================================

ALTER TABLE agents ADD COLUMN context_window INTEGER;
