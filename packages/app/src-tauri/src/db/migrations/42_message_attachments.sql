-- message_attachments：聊天附件分页的「提取后按块文本」存储
--
-- 背景：聊天附件路径（chat_cmd materialize）原本把整篇提取文本塞进单个 Text block
-- 一次性发给 LLM，大 PDF（>1M）提取出数万~十几万 tokens → 超窗口被裁 / 超请求上限
-- 被拒 → 表现为"读不到"。read_file 路径有行分页所以无此问题，痛点专属附件路径。
--
-- 治本（Phase A）：大附件按块切（PDF 按页 / xlsx 按 sheet / docx 按 token 段），
-- 只把首页注入 LLM，后续页由 read_attachment_page 工具按 (message_id, page) 取。
-- 本表只存「提取后的块文本」，不存原始字节——FK CASCADE 跟消息生命周期一致、
-- 无孤儿/GC、list messages 不带大字段、DB 不胀。
CREATE TABLE message_attachments (
  id         TEXT PRIMARY KEY,            -- "{message_id}:{idx}"
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  idx        INTEGER NOT NULL,            -- 0-based 块序（page 参数 = idx + 1）
  name       TEXT NOT NULL,               -- 原文件名（含扩展名）
  kind       TEXT NOT NULL,               -- pdf / docx / spreadsheet ...
  label      TEXT NOT NULL,               -- "第3页" / "Sheet:销售" / "第2段"
  text       TEXT NOT NULL,               -- 该块的提取正文
  token_est  INTEGER NOT NULL DEFAULT 0   -- 该块 token 估算（estimate_tokens）
);

CREATE INDEX idx_msg_att_msg ON message_attachments(message_id, idx);
