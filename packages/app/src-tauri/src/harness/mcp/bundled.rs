//! 内置 MCP 运行时：bundled server 的元数据查表 + resource_dir 路径解析。
//!
//! `runtime_kind = Bundled` 的 server 不走系统 PATH / npx，而是用 IcePaw 安装包自带的
//! Node 运行时 + 预打包 node_modules。这样在 GFW 环境 / npm 缓存损坏时也能零网络启动
//! （生产曾出现 `server-sequential-thinking` 缺传递依赖 `zod` 而启动失败）。
//!
//! 资源布局（由 `tauri.conf.json` 的 `bundle.resources` 打包进安装目录）：
//! ```text
//! <resource_dir>/resources/mcp-runtime/
//!   node/node.exe                                            ← 内置 Node win-x64
//!   node_modules/@modelcontextprotocol/server-sequential-thinking/dist/index.js
//!   node_modules/@modelcontextprotocol/server-memory/dist/index.js
//!   node_modules/zod/...                                     ← 关键传递依赖
//! ```
//!
//! bundled server 在 DB 里 command 存占位 `"node"`、args 存「用户可配参数」（不含包名/入口）。
//! `McpServerManager::start_server` 解析时：把 command 换成 `node_exe()` 的绝对路径，
//! 并把对应包的 entry script prepend 到 args 前面（见 [`spec_for`]）。

use std::path::PathBuf;

use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

/// bundled runtime 在 resource_dir 下的相对根。
const RUNTIME_REL: &str = "resources/mcp-runtime";

/// env 模板里的占位符：由 start_server 替换为 app_data_dir 下可写的 memory 数据文件路径。
/// （node_modules 是只读的，memory server 不能往那里写。）
pub const MEMORY_FILE_PLACEHOLDER: &str = "{memory_data_file}";

/// 单个 bundled server 的固定元数据（server_id → 包目录 / ESM 入口 / 固定 env 模板）。
pub struct BundledSpec {
    /// node_modules 下的包目录，如 `@modelcontextprotocol/server-sequential-thinking`
    pub package_dir: &'static str,
    /// 相对包目录的 ESM 入口脚本，如 `dist/index.js`
    pub entry_script: &'static str,
    /// 固定 env 模板（值里可含 [`MEMORY_FILE_PLACEHOLDER`]，由调用方替换）。
    /// 用户在 DB 里声明的 env 会覆盖这些（同 key 后者胜）。
    pub env_template: &'static [(&'static str, &'static str)],
}

/// 2 个内置 bundled server 的查表。未命中返回 None（调用方报错）。
pub fn spec_for(server_id: &str) -> Option<&'static BundledSpec> {
    static SPECS: &[(&str, BundledSpec)] = &[
        (
            "builtin-thinking",
            BundledSpec {
                package_dir: "@modelcontextprotocol/server-sequential-thinking",
                entry_script: "dist/index.js",
                env_template: &[],
            },
        ),
        (
            "builtin-memory",
            BundledSpec {
                package_dir: "@modelcontextprotocol/server-memory",
                // memory server 读 MEMORY_FILE_PATH 决定持久化位置（默认会写到 npx 缓存深处、
                // 清缓存即丢）。指向 app_data_dir 下可写文件，跨会话保留知识图谱。
                entry_script: "dist/index.js",
                env_template: &[("MEMORY_FILE_PATH", MEMORY_FILE_PLACEHOLDER)],
            },
        ),
    ];
    SPECS
        .iter()
        .find(|(id, _)| *id == server_id)
        .map(|(_, s)| s)
}

/// bundled runtime 根目录（resource_dir 下）。
///
/// dev/prod 都用 `BaseDirectory::Resource` 解析；dev 模式下若该路径不存在，
/// 回退到编译期 `CARGO_MANIFEST_DIR`（即 src-tauri 源码目录），兼容开发态资源未打包的情况。
/// release 构建不启用回退（CARGO_MANIFEST_DIR 是构建机路径，用户机器上不存在）。
pub fn runtime_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let resolved = app
        .path()
        .resolve(RUNTIME_REL, tauri::path::BaseDirectory::Resource)
        .map_err(|e| AppError::Tauri(format!("解析 mcp_runtime_dir 失败: {e}")))?;
    if resolved.exists() || !cfg!(debug_assertions) {
        return Ok(resolved);
    }
    // dev 回退：src-tauri 源码目录下的 resources/mcp-runtime
    let fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(RUNTIME_REL);
    if fallback.exists() {
        Ok(fallback)
    } else {
        Ok(resolved)
    }
}

/// 内置 node.exe 绝对路径。
pub fn node_exe(app: &AppHandle) -> AppResult<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    Ok(runtime_dir(app)?.join("node").join(name))
}

/// 某个 bundled server 的 entry script 绝对路径。
pub fn entry_script(app: &AppHandle, spec: &BundledSpec) -> AppResult<PathBuf> {
    Ok(runtime_dir(app)?
        .join("node_modules")
        .join(spec.package_dir)
        .join(spec.entry_script))
}

/// 校验 bundled runtime 已就位：node.exe + 全部 entry script 存在。
///
/// 启动时（或 retry 前）调用；失败应仅 warn + 把 server 标 Failed，不阻塞应用启动。
pub fn verify(app: &AppHandle) -> AppResult<()> {
    let node = node_exe(app)?;
    if !node.exists() {
        return Err(AppError::Internal(format!(
            "内置 Node 缺失: {}（请运行 pnpm prepare:mcp 准备内置 MCP 运行时）",
            node.display()
        )));
    }
    for id in ["builtin-thinking", "builtin-memory"] {
        let spec = spec_for(id).expect("bundled server id 拼写错误");
        let entry = entry_script(app, spec)?;
        if !entry.exists() {
            return Err(AppError::Internal(format!(
                "内置 MCP 包缺失: {}（请运行 pnpm prepare:mcp）",
                entry.display()
            )));
        }
    }
    Ok(())
}

/// 为 memory server 解析可写数据文件路径：`<app_data_dir>/mcp-memory/memory.jsonl`。
/// 复用 logging::data_dir（app_data_dir）。目录不存在则创建。
pub fn memory_data_file(app: &AppHandle) -> AppResult<String> {
    let dir = crate::logging::data_dir(app)?.join("mcp-memory");
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "创建 memory 数据目录失败: {e}"
        )))
    })?;
    // node 在 Windows 上接受正斜杠；统一用正斜杠避免反斜杠转义问题
    Ok(dir
        .join("memory.jsonl")
        .to_string_lossy()
        .replace('\\', "/"))
}

/// 把 spec 的 env 模板渲染成具体 env（替换 {memory_data_file} 占位符）。
/// 返回 serde_json object，供 start_server 与用户 env 合并后传给 spawn。
pub fn render_env_template(
    spec: &BundledSpec,
    app: &AppHandle,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    for (k, v) in spec.env_template {
        let rendered = if v.contains(MEMORY_FILE_PLACEHOLDER) {
            let file = memory_data_file(app)?;
            v.replace(MEMORY_FILE_PLACEHOLDER, &file)
        } else {
            (*v).to_string()
        };
        map.insert((*k).to_string(), json!(rendered));
    }
    Ok(map)
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_for_known_ids() {
        assert!(spec_for("builtin-thinking").is_some());
        assert!(spec_for("builtin-memory").is_some());
    }

    #[test]
    fn spec_for_unknown_id_returns_none() {
        assert!(spec_for("builtin-playwright").is_none());
        assert!(spec_for("nonexistent").is_none());
    }

    #[test]
    fn memory_spec_has_memory_file_path_env() {
        let spec = spec_for("builtin-memory").unwrap();
        let keys: Vec<_> = spec.env_template.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"MEMORY_FILE_PATH"));
        let (_, v) = spec
            .env_template
            .iter()
            .find(|(k, _)| *k == "MEMORY_FILE_PATH")
            .unwrap();
        assert!(v.contains(MEMORY_FILE_PLACEHOLDER));
    }

    #[test]
    fn thinking_has_no_env_template() {
        assert!(spec_for("builtin-thinking")
            .unwrap()
            .env_template
            .is_empty());
    }

    #[test]
    fn all_entries_use_dist_index_js() {
        for id in ["builtin-thinking", "builtin-memory"] {
            assert_eq!(spec_for(id).unwrap().entry_script, "dist/index.js");
        }
    }
}
