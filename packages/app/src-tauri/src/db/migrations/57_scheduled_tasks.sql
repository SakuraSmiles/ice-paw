-- 57_scheduled_tasks.sql — 定时任务（0.9.3）：任务实体 + 执行记录两表
-- 设计真相源 docs/scheduled-tasks-design.md（2026-09-19 拍板六点）。
-- 载体 = 仅专属会话懒建（target_conv_id 首跑物化时回写）；结果可配置转发投递
-- （deliver_to_conv_id 走 MA-3 通道）；错过策略任务级（run_once 补跑一次 / skip 顺延）。
-- 时间戳全 UTC 存储（datetime('now')，对齐全 DB 惯例——parseDbTime 按 UTC 解析转本地显示）；
-- 调度计算按本地语义（用户输入 09:00 = 本地九点），存库时 Local→UTC 转换。
-- 一次性任务完成后置 NULL 并禁用（NULL 不命中 due 扫描）。

CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    -- once | daily | weekly | interval | cron
    schedule_kind TEXT NOT NULL,
    -- 档位 JSON 载荷：once{"at"} / daily{"time"} / weekly{"weekdays",[0-6],"time"} /
    -- interval{"minutes"} / cron{"expr"}
    schedule_data TEXT NOT NULL,
    prompt TEXT NOT NULL,
    -- NULL = 专属会话懒建；创建后不可改绑（换载体 = 新任务）
    target_conv_id TEXT,
    -- run_once | skip（默认 run_once）
    miss_policy TEXT NOT NULL DEFAULT 'run_once',
    -- NULL = 不转发；转发走 MA-3 deliver（目标会话按其收件政策消费）
    deliver_to_conv_id TEXT,
    -- 无人值守预授权：none（默认，Confirm 照常弹卡 2 分钟超时拒绝）/
    -- commands（run_command 免问，委派同款 seed）/ all（本任务会话全部
    -- Confirm 工具免问——巡检类重工具任务；屏幕家族仍走通道治理不进预授权）
    preauth TEXT NOT NULL DEFAULT 'none',
    enabled INTEGER NOT NULL DEFAULT 1,
    next_run TEXT,
    last_run_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS task_runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    -- 载体会话（懒建前可短暂为 NULL——run 行在会话确定后插入，正常恒非空）
    conv_id TEXT,
    status TEXT NOT NULL,
    summary TEXT,
    error TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_task_runs_task ON task_runs(task_id, started_at DESC);
