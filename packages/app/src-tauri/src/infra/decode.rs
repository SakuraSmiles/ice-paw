//! 统一字节解码：UTF-8 优先 → Windows 系统代码页 → lossy
//!
//! 供 `read_file`、`run_command`、`git` 等工具使用，确保外部子进程的输出
//! 在任意平台都不会变成乱码。
//!
//! 跨平台策略（全部基于系统事实，不做启发式猜测）：
//! - **UTF-8 优先**：UTF-8 自校验极强，误判≈0。Linux/Mac locale 基本都
//!   UTF-8，直接命中；Windows 上现代 CLI（git/node/rust/go）也常命中。
//! - **Windows 系统代码页兜底**：UTF-8 解不开时，用 `MultiByteToWideChar`
//!   配 `CP_ACP`（=0，当前系统 ANSI 代码页）解码——自动适配任意语言版本
//!   的 Windows（中文=GBK、英文=1252、日文=Shift-JIS…），不再硬编码 GBK。
//! - **lossy 兜底**：极少见。

/// 解码结果
pub(crate) struct DecodedText {
    pub text: String,
    /// 实际使用的编码：utf-8 / system-ansi / utf-8-lossy
    pub encoding: &'static str,
}

/// Windows：用系统当前 ANSI 代码页（`CP_ACP`=0）把字节解为 UTF-16 再转 String。
///
/// 走 Windows 原生转换表，覆盖所有代码页（含中文 GBK、日文 Shift-JIS、
/// 韩文 EUC-KR、西欧 1252 等），且自动随系统语言切换，无需硬编码假设。
/// 解码失败（无效序列等）返回 `None`，由上层走 lossy。
#[cfg(windows)]
fn decode_system_ansi(bytes: &[u8]) -> Option<String> {
    use std::ptr;
    use windows_sys::Win32::Globalization::MultiByteToWideChar;

    const CP_ACP: u32 = 0; // 0 = 当前系统 ANSI 代码页
    const MB_ERR_INVALID_CHARS: u32 = 0x0000_0008;

    if bytes.is_empty() {
        return Some(String::new());
    }
    // SAFETY：MultiByteToWideChar 是 raw FFI：
    // - bytes.as_ptr() + bytes.len() 指向调用期间有效的字节缓冲区；
    // - 第一遍 cchWideChar=0、目标传 null：仅探测所需长度（MSDN 允许）；
    // - 第二遍 wstr 是 len 元素的有效 u16 缓冲区，容量足够写入。
    let (written, mut wstr) = unsafe {
        // 第一遍：cchWideChar=0，取所需 UTF-16 长度
        let len = MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            ptr::null_mut(),
            0,
        );
        if len <= 0 {
            return None;
        }
        // 第二遍：实际转换
        let mut wstr = vec![0u16; len as usize];
        let written = MultiByteToWideChar(
            CP_ACP,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            wstr.as_mut_ptr(),
            len,
        );
        (written, wstr)
    };
    if written <= 0 {
        return None;
    }
    wstr.truncate(written as usize);
    String::from_utf16(&wstr).ok()
}

/// 字节 → 文本：UTF-8 优先，失败则（Windows）系统 ANSI 代码页，兜底 lossy。
pub(crate) fn decode_bytes(bytes: &[u8]) -> DecodedText {
    // 1. UTF-8（Linux/Mac 几乎必命中；Windows 现代工具也常命中）
    if let Ok(s) = std::str::from_utf8(bytes) {
        return DecodedText {
            text: s.to_string(),
            encoding: "utf-8",
        };
    }

    // 2. Windows：系统 ANSI 代码页（CP_ACP 自动适配任意语言版本）
    #[cfg(windows)]
    {
        if let Some(s) = decode_system_ansi(bytes) {
            return DecodedText {
                text: s,
                encoding: "system-ansi",
            };
        }
    }

    // 3. 兜底：UTF-8 lossy
    DecodedText {
        text: String::from_utf8_lossy(bytes).into_owned(),
        encoding: "utf-8-lossy",
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
    fn invalid_utf8_never_panics_and_nonempty() {
        // 非 UTF-8 字节：Windows 走系统代码页（常解出字符），其他平台 lossy
        // 成替换字符；两条路径都不 panic 且返回非空文本。
        let r = decode_bytes(&[0xC0, 0xAF]); // overlong UTF-8，无效
        assert!(!r.text.is_empty());
    }

    #[test]
    fn empty_bytes() {
        let r = decode_bytes(b"");
        assert_eq!(r.text, "");
        assert_eq!(r.encoding, "utf-8");
    }
}
