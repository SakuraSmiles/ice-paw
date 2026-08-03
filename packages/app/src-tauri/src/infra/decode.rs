//! 统一字节解码：UTF-8 优先 → 系统编码兜底 → lossy
//!
//! 供 `read_file`、`run_command` 等工具使用，确保 Windows GBK 等
//! 非 UTF-8 内容不会变成乱码。

/// 解码结果
pub(crate) struct DecodedText {
    pub text: String,
    /// 实际使用的编码：utf-8 / gbk / utf-8-lossy
    pub encoding: &'static str,
}

/// 字节 → 文本：UTF-8 优先，失败则尝试系统编码（Windows GBK），兜底 lossy
pub(crate) fn decode_bytes(bytes: &[u8]) -> DecodedText {
    // 1. 尝试 UTF-8
    if let Ok(s) = std::str::from_utf8(bytes) {
        return DecodedText { text: s.to_string(), encoding: "utf-8" };
    }

    // 2. Windows：尝试 GBK（中文 Windows 默认编码）
    #[cfg(windows)]
    {
        let (cow, _encoding, had_errors) = encoding_rs::GBK.decode(bytes);
        if !had_errors {
            return DecodedText { text: cow.into_owned(), encoding: "gbk" };
        }
        // GBK 也有错误 → 用 lossy GBK
        return DecodedText { text: cow.into_owned(), encoding: "gbk-lossy" };
    }

    // 3. 兜底：UTF-8 lossy
    #[cfg(not(windows))]
    {
        DecodedText {
            text: String::from_utf8_lossy(bytes).into_owned(),
            encoding: "utf-8-lossy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_valid_utf8() {
        let r = decode_bytes("hello 中文".as_bytes());
        assert_eq!(r.encoding, "utf-8");
        assert_eq!(r.text, "hello 中文");
    }

    #[test]
    fn decodes_ascii() {
        let r = decode_bytes(b"Hello World");
        assert_eq!(r.encoding, "utf-8");
        assert_eq!(r.text, "Hello World");
    }

    #[test]
    fn lossy_on_invalid_utf8_non_windows() {
        let r = decode_bytes(&[0xC0, 0xAF]); // invalid UTF-8
        // On Windows: tries GBK first; on non-Windows: direct lossy
        assert!(!r.text.is_empty());
    }
}
