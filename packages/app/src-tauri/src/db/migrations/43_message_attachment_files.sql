-- 附件原始字节留存（Phase B 视觉/出图用）。
--
-- 仅当文本提取为空（total_tokens == 0，疑似扫描件/纯图片/加密 PDF）时才写一行——
-- 视觉工具 view_attachment_image 按需读字节渲染页面。文本提取成功的附件不留存字节
-- （agent 已有文本，无需视觉），避免给 DB 塞无用大 BLOB。
--
-- 一文件一行；生命周期跟 messages 外键 CASCADE，删消息自动清，无 GC。
CREATE TABLE message_attachment_files (
    id          TEXT PRIMARY KEY,            -- "{message_id}:{file_idx}"
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,            -- 文件在消息内的序号（0-based）
    name        TEXT NOT NULL,
    ext         TEXT NOT NULL,
    bytes       BLOB NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_msg_att_file_msg ON message_attachment_files(message_id);
