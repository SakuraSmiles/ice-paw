//! 路径归一与成员判定 —「路径是否在授权范围内」的单一真相源。
//!
//! 为什么存在：Windows `canonicalize()` 返回 `\\?\C:\...` verbatim 形式，而工具
//! 参数里的原始路径没有该前缀；目标路径不存在（新建文件/未建目录）时
//! canonicalize 失败，旧兜底直接拿原始串 `starts_with` 比较 → verbatim 前缀 /
//! 大小写 / `..` 三类失配：workspace 内不存在路径 100% 误弹审批（0.3.5 生产
//! 反馈），且 `C:\ws\..\..\x` 可借字符串前缀静默越权放行（安全级）。
//!
//! 提供**不碰文件系统**的词法归一（`lexical_normalize`）+ 平台感知的组件级
//! 成员判定（`path_within`），workspace 免授权 / 路径白名单 / 会话授权记忆
//! 三处共用。
//!
//! ⚠️ 不变式：任何「路径是否在授权范围内」的判断必须经由 `path_within`
//! （或其封装），不得手搓字符串 `starts_with`——组件级比较才不会被
//! `/ws-secret` 骗过 `/ws` 前缀，词法消解才不会被 `..` 穿越骗过。

use std::path::{Component, Path, PathBuf};

/// 剥除 Windows verbatim 前缀：`\\?\C:\x` → `C:\x`、`\\?\UNC\srv\sh` → `\\srv\sh`。
/// canonicalize() 在 Windows 恒返回 verbatim 形，与工具参数原始路径不同形，
/// 直接比较恒 false（workspace 误弹审批的根因）。非 verbatim 路径原样返回。
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

/// 词法归一（不碰文件系统）：绝对化 + 剥 verbatim + 消解 `.`/`..` 组件。
/// 相对路径按当前工作目录绝对化（授权判定需要统一坐标系才可比较）。
/// 用于目标路径不存在的场景（canonicalize 不可用），与 canonicalize 结果
/// 归一到同一形态。
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        strip_verbatim(path)
    } else {
        // std::path::absolute（1.79+）：纯词法绝对化，不触碰文件系统
        match std::path::absolute(path) {
            Ok(p) => strip_verbatim(&p),
            Err(_) => strip_verbatim(path),
        }
    };
    // 组件级消解 ./..：`..` 只回退 Normal 组件，不许弹出盘符/根
    // （否则 C:\..\x 会塌成驱动器相对路径 C:x，比较语义错乱）。
    let mut comps: Vec<Component<'_>> = Vec::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                while matches!(comps.last(), Some(Component::Normal(_))) {
                    comps.pop();
                }
            }
            other => comps.push(other),
        }
    }
    let mut out = PathBuf::new();
    for c in comps {
        out.push(c);
    }
    out
}

/// target 是否位于 root 内（含 root 本身）。
///
/// - 两侧先 `lexical_normalize`（canonicalize 结果的 verbatim 前缀在此剥除）
/// - 组件级前缀比较：`/ws` 不覆盖 `/ws-secret`（字符串 starts_with 的经典漏洞）
/// - Windows 大小写不敏感（NTFS 默认），Unix 区分大小写
pub fn path_within(target: &Path, root: &Path) -> bool {
    let t = lexical_normalize(target);
    let r = lexical_normalize(root);
    let tc: Vec<_> = t.components().collect();
    let rc: Vec<_> = r.components().collect();
    if rc.len() > tc.len() {
        return false;
    }
    tc[..rc.len()]
        .iter()
        .zip(rc.iter())
        .all(|(a, b)| comp_eq(a, b))
}

/// 会话授权记忆的归一缓存键：词法归一（+ Windows 大小写折叠——同一文件
/// 不同写法不应重复弹窗；Unix 保持区分，两文件仅大小写不同就是两个文件）。
pub fn auth_cache_key(path: &str) -> String {
    let normalized = lexical_normalize(Path::new(path)).to_string_lossy().into_owned();
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

#[cfg(windows)]
fn comp_eq(a: &Component<'_>, b: &Component<'_>) -> bool {
    a.as_os_str()
        .to_string_lossy()
        .to_lowercase()
        == b.as_os_str().to_string_lossy().to_lowercase()
}

#[cfg(not(windows))]
fn comp_eq(a: &Component<'_>, b: &Component<'_>) -> bool {
    a == b
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- 组件级成员判定（平台无关） -----

    #[test]
    fn within_subpath() {
        assert!(path_within(Path::new("/ws/sub/file.txt"), Path::new("/ws")));
        assert!(path_within(Path::new("/ws/sub/deep/file.rs"), Path::new("/ws")));
    }

    #[test]
    fn within_equal_counts_as_inside() {
        assert!(path_within(Path::new("/ws"), Path::new("/ws")));
        assert!(path_within(Path::new("/ws/"), Path::new("/ws"))); // 尾斜杠
    }

    #[test]
    fn rejects_sibling_prefix() {
        // 字符串 starts_with 的经典漏洞：/ws-secret 不属于 /ws
        assert!(!path_within(Path::new("/ws-secret/file.txt"), Path::new("/ws")));
    }

    #[test]
    fn rejects_outside() {
        assert!(!path_within(Path::new("/etc/passwd"), Path::new("/ws")));
        assert!(!path_within(
            Path::new("/etc/passwd"),
            Path::new("/a/b/c/d/e") // root 比 target 长
        ));
    }

    // ----- .. 穿越（安全级：词法消解后才判归属） -----

    #[test]
    fn dotdot_traversal_rejected() {
        // 终点不存在 → canonicalize 不可用的场景，词法归一后判定
        assert!(!path_within(
            Path::new("/ws/../../no_such_top_dir/secret.txt"),
            Path::new("/ws")
        ));
        assert!(!path_within(
            Path::new("/ws/sub/../../../no_such_dir"),
            Path::new("/ws")
        ));
    }

    #[test]
    fn dotdot_stays_inside_when_resolved_inside() {
        // 消解后仍在 root 内 → 放行（合法等价写法，不该误弹）
        assert!(path_within(Path::new("/ws/sub/../file.txt"), Path::new("/ws")));
    }

    // ----- 词法归一 -----

    #[test]
    fn normalize_removes_curdir() {
        // 两侧同过归一再比（平台中立：`/ws` 在 Windows 非绝对路径，
        // 会被 absolute 解析到当前盘符——断言的是「语义等价」而非字面值）
        assert_eq!(
            lexical_normalize(Path::new("/ws/./sub/./file")),
            lexical_normalize(Path::new("/ws/sub/file"))
        );
    }

    #[test]
    fn normalize_resolves_dotdot() {
        assert_eq!(
            lexical_normalize(Path::new("/ws/sub/../other")),
            lexical_normalize(Path::new("/ws/other"))
        );
    }

    #[test]
    fn normalize_relative_becomes_absolute() {
        let n = lexical_normalize(Path::new("some_file.txt"));
        assert!(n.is_absolute());
        assert!(n.ends_with("some_file.txt"));
    }

    // ----- Windows verbatim / 大小写（回归根因） -----

    #[cfg(windows)]
    #[test]
    fn verbatim_root_matches_raw_target() {
        // 0.3.5 生产 bug 回归：workspace 走 canonicalize 得 \\?\C:\ws，
        // 新建文件（不存在）走原始串——旧 starts_with 恒 false → 误弹审批
        assert!(path_within(
            Path::new(r"C:\ws\new_dir\new_file.txt"),
            Path::new(r"\\?\C:\ws")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn case_insensitive_on_windows() {
        assert!(path_within(Path::new(r"c:\WS\File.txt"), Path::new(r"C:\ws")));
        assert!(path_within(
            Path::new(r"C:\ws\FILE.txt"),
            Path::new(r"\\?\c:\Ws")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn strip_verbatim_forms() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Users\x")),
            PathBuf::from(r"C:\Users\x")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\srv\share")),
            PathBuf::from(r"\\srv\share")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"C:\plain")),
            PathBuf::from(r"C:\plain")
        );
    }

    #[cfg(windows)]
    #[test]
    fn dotdot_not_pop_past_drive_root() {
        // C:\..\x 不得塌成驱动器相对路径 C:x
        let n = lexical_normalize(Path::new(r"C:\..\x"));
        assert_eq!(n, PathBuf::from(r"C:\x"));
    }

    // ----- 会话授权缓存键 -----

    #[test]
    fn auth_cache_key_normalized() {
        assert_eq!(
            auth_cache_key("/ws/./sub/../file"),
            auth_cache_key("/ws/file")
        );
    }

    #[cfg(windows)]
    #[test]
    fn auth_cache_key_case_folded_on_windows() {
        assert_eq!(auth_cache_key(r"C:\Ws\File"), auth_cache_key(r"c:\ws\file"));
    }
}
