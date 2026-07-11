//! Stronghold 封装层
//!
//! 设计目标：
//! - 全局维护一个 `Stronghold` 实例（snapshot 落盘到 `app_data_dir/stronghold.hold`）
//! - 密码（用于加密 snapshot）的来源：当前 M2 使用固定串，
//!   Phase 2 再接入 OS keyring / 用户密码派生
//! - 所有 API 接收 `&AppHandle`，内部从 Tauri 状态取 `CryptoState`
//!
//! 注意：调用方应保证**只有一个** stronghold 实例，否则会出现快照冲突。
//!
//! ---
//! ## 关于密码长度
//!
//! Stronghold 的 `KeyProvider::try_from(Zeroizing<Vec<u8>>)` 要求密码**恰好 32 字节**，
//! 否则返回 `MemoryError::NCSizeNotAllowed`（错误信息显示为
//! "illegal non-contiguous size"），导致 `Stronghold::new` 直接失败。
//!
//! 这个限制与平台无关（WSL / Linux / Windows / macOS 都会失败），并不是 WSL/GPU/D-Bus
//! 引起的问题。原实现里 `DEFAULT_PASSWORD = "icepaw-default-vault-key-v1"` 是 27 字节，
//! 因此首次启动就会崩溃。
//!
//! 这里采用 stronghold 文档推荐的 `KeyProvider::with_passphrase_hashed_blake2b` 思路：
//! 在 Rust 端用 blake2b256 把任意长度的 passphrase 派生到 32 字节，再传给 `Stronghold::new`。
//! 这样：
//! - 字符串可以随便写（不会再触发"密码必须 32 字节"的 footgun）
//! - 与 stronghold 内部的 key 派生方式一致
//! - Phase 2 接入 OS keyring 时只需替换 passphrase 的来源，hash 步骤不变

use std::sync::{Arc, Mutex, MutexGuard};

use blake2::{Blake2b, Digest};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_stronghold::stronghold::Stronghold;
use tracing::{info, warn};

use crate::error::{AppError, AppResult};

/// 内部状态：包装 plugin 的 `Stronghold`，对它做并发安全包装
pub struct CryptoState {
    pub inner: Arc<Mutex<Stronghold>>,
}

/// 储存的 value 形状
#[derive(Debug, Serialize, Deserialize)]
struct ApiKeyRecord {
    #[serde(rename = "apiKey")]
    api_key: String,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none", default)]
    base_url: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

/// 客户端名常量（唯一 client）
const CLIENT_NAME: &[u8] = b"icepaw";

/// 全局默认密码（强约束场景请改成用户解锁派生）
///
/// 当前 M2 阶段：先用固定串 + 文件权限 + WAL 加密缓解（强于 plugin-store），
/// Phase 2 再接入 OS keyring 派生。
///
/// **不要**改成任意 32 字节后以为大功告成 —— blake2b 派生层会把任意长度
/// 规范化到 32 字节，写啥都行。Phase 2 改成从 keyring 读字节即可。
const DEFAULT_PASSPHRASE: &str = "icepaw-default-vault-key-v1";

/// blake2b-256 输出长度（字节）= Stronghold 要求的 key 长度
const STRONGHOLD_KEY_LEN: usize = 32;

/// 把任意长度 passphrase 派生为 32 字节 stronghold key
///
/// 使用 blake2b256，与 stronghold 内部 `KeyProvider::with_passphrase_hashed_blake2b`
/// 完全等价（同样的 digest + 同样的 32 字节输出），只是我们在 `Stronghold::new` 之外
/// 提前算好 hash 喂进去，避开 tauri-plugin-stronghold wrapper 那个
/// `KeyProvider::try_from(Zeroizing<Vec<u8>>)` 的 32 字节硬约束。
///
/// `pub` 是为了让 `lib.rs` 的 stronghold plugin Builder 也能复用同一份 hash 逻辑
/// （前端 JS API 走的也是同一段路径，避免触发同样的 length 错误）。
pub fn derive_stronghold_key(passphrase: &[u8]) -> [u8; STRONGHOLD_KEY_LEN] {
    type Blake2b256 = Blake2b<blake2::digest::consts::U32>;
    let mut hasher = Blake2b256::new();
    hasher.update(passphrase);
    let out = hasher.finalize();
    let mut key = [0u8; STRONGHOLD_KEY_LEN];
    key.copy_from_slice(&out);
    key
}

/// 取出 CryptoState 内部的 Arc 克隆（避免对 AppHandle 的长借用）
fn inner_arc(app: &AppHandle) -> Arc<Mutex<Stronghold>> {
    app.state::<CryptoState>().inner.clone()
}

/// 初始化 stronghold：放 `app.manage(CryptoState)`
///
/// 幂等：重复调用仅返回现有实例。
pub fn init(app: &AppHandle) -> AppResult<()> {
    if app.try_state::<CryptoState>().is_some() {
        return Ok(());
    }

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Tauri(format!("解析 app_data_dir 失败: {e}")))?;
    std::fs::create_dir_all(&dir)?;
    let snapshot = dir.join("stronghold.hold");
    info!(target: "ice_paw.crypto", "Stronghold snapshot: {}", snapshot.display());

    // 派生 32 字节 key（详见模块顶部说明）
    let key = derive_stronghold_key(DEFAULT_PASSPHRASE.as_bytes());
    let sh = Stronghold::new(&snapshot, key.to_vec())
        .map_err(|e| AppError::Stronghold(format!("Stronghold::new: {e}")))?;
    let sh_arc = Arc::new(Mutex::new(sh));

    {
        let guard: MutexGuard<'_, Stronghold> = sh_arc.lock().expect("init: mutex poisoned");
        match guard.get_client(CLIENT_NAME) {
            Ok(_) => info!(target: "ice_paw.crypto", "Stronghold client 已存在"),
            Err(_) => {
                guard
                    .create_client(CLIENT_NAME)
                    .map_err(|e| AppError::Stronghold(format!("create_client: {e}")))?;
                info!(target: "ice_paw.crypto", "Stronghold client 已创建");
            }
        }
        if let Err(e) = guard.save() {
            warn!(target: "ice_paw.crypto", "首次落盘失败（非致命）: {e}");
        }
    } // guard dropped here

    app.manage(CryptoState { inner: sh_arc });
    Ok(())
}

/// 储存 (api_key, base_url) 到 vault
pub fn store_api_key(
    app: &AppHandle,
    agent_id: &str,
    api_key: &str,
    base_url: Option<&str>,
) -> AppResult<()> {
    if agent_id.is_empty() {
        return Err(AppError::Validation("agent_id 不能为空".into()));
    }

    let sh_arc = inner_arc(app);
    let sh = sh_arc.lock().expect("store: mutex poisoned");

    let record = ApiKeyRecord {
        api_key: api_key.to_string(),
        base_url: base_url.map(String::from),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec(&record)?;

    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    store
        .insert(agent_id.as_bytes().to_vec(), bytes, None)
        .map_err(|e| AppError::Stronghold(format!("store.insert: {e}")))?;
    sh.save()?;
    Ok(())
    // sh (guard) dropped here at function exit
}

/// 取回 (api_key, base_url)
pub fn fetch_api_key(
    app: &AppHandle,
    agent_id: &str,
) -> AppResult<(String, Option<String>)> {
    let sh_arc = inner_arc(app);
    let sh = sh_arc.lock().expect("fetch: mutex poisoned");

    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    let raw = store
        .get(agent_id.as_bytes())
        .map_err(|e| AppError::Stronghold(format!("store.get: {e}")))?
        .ok_or_else(|| AppError::NotFound {
            resource: "api_key",
            id: agent_id.to_string(),
        })?;

    let record: ApiKeyRecord = serde_json::from_slice(&raw)?;
    Ok((record.api_key, record.base_url))
}

/// 删除该 agent 对应的密钥（找不到不报错）
pub fn delete_api_key(app: &AppHandle, agent_id: &str) -> AppResult<()> {
    let sh_arc = inner_arc(app);
    let sh = sh_arc.lock().expect("delete: mutex poisoned");
    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    match store.delete(agent_id.as_bytes()) {
        Ok(_) => {}
        Err(e) => {
            warn!(target: "ice_paw.crypto", "delete 返回错误（容错忽略）: {e}");
        }
    }
    let _ = sh.save();
    Ok(())
}

/// 列出所有已存 agent_id
pub fn list_agent_ids(app: &AppHandle) -> AppResult<Vec<String>> {
    let sh_arc = inner_arc(app);
    let sh = sh_arc.lock().expect("list: mutex poisoned");
    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    let keys: Vec<Vec<u8>> = store
        .keys()
        .map_err(|e| AppError::Stronghold(format!("store.keys: {e}")))?;
    Ok(keys
        .into_iter()
        .map(|k| String::from_utf8_lossy(&k).into_owned())
        .collect())
}

/// 检查某个 agent 是否已存 api_key
pub fn has_api_key(app: &AppHandle, agent_id: &str) -> AppResult<bool> {
    let sh_arc = inner_arc(app);
    let sh = sh_arc.lock().expect("has: mutex poisoned");
    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    Ok(store
        .contains_key(agent_id.as_bytes())
        .map_err(|e| AppError::Stronghold(format!("store.contains_key: {e}")))?)
}
