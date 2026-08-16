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
//! ## 启动恢复语义（关键不变量）
//!
//! `Stronghold::new(path, key)` 在底层做了：
//!   1. `iota_stronghold::Stronghold::default()` —— 创建一个空内存结构
//!      （`self.snapshot = Snapshot::default()`、`self.clients = HashMap::new()`）
//!   2. 当 `path` 存在时，调用 `load_snapshot(&kp, &path)` —— **只**把整份快照
//!      加载到 `self.snapshot.states`，**不会**把它恢复到 `self.clients` 这个 HashMap。
//!
//! 因此 `Stronghold::new` 之后，`self.snapshot` 与 `self.clients` 是分离的：
//!   - `get_client(path)` 只查 `self.clients`，若 client 还没被恢复到内存
//!     就返回 `ClientError::ClientDataNotPresent`。
//!   - `load_client(path)` 会主动从 `self.snapshot.states` 中把对应 client 的
//!     `(keystore, vault, store)` **恢复到** `self.clients`，并返回 client 句柄。
//!
//! ⚠️ **历史陷阱**：早期实现用 `get_client + create_client + save` 的模式：
//!   `get_client` 误判 client "不存在" → `create_client` 在 `self.clients` 里
//!   插入一个**空的** client → `save()` 走 `commit_with_keyprovider`，遍历
//!   `self.clients` 把当前（含空 client）的状态写回 snapshot 文件 ——
//!   **覆盖**原本已存在的 client 数据。重启后所有 agent 的 api_key 因此丢失。
//!
//! ✅ **正确做法**（见 `init`）：先 `load_client`，只有当它返回
//! `ClientDataNotPresent`（说明 snapshot 里确实没有该 client）时，才走
//! `create_client + save` 首次落盘。任何其他错误原样向上抛。
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
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use iota_stronghold::ClientError as ShClientError;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_stronghold::stronghold::Stronghold;
use tracing::info;

use crate::error::{AppError, AppResult};

/// 内部状态：包装 plugin 的 `Stronghold`，对它做并发安全包装
pub struct CryptoState {
    pub inner: Arc<Mutex<Stronghold>>,
}

impl CryptoState {
    /// 获取 Stronghold 锁，从 Mutex 毒化状态自动恢复。
    ///
    /// 当持有锁的线程 panic 时，`std::sync::Mutex` 被标记为 "poisoned"。
    /// 我们选择恢复数据而非 panic 传播，因为：
    /// - Stronghold 内部是自包含的 HashMap，毒化不会导致数据损坏
    /// - 单次 panic 不应让整个应用的密钥存储不可用
    ///
    /// 与 `ChatState::lock()` 同模式。
    fn lock_sh(&self) -> MutexGuard<'_, Stronghold> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
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
/// `pub(crate)` —— 仅 crate 内使用。原先 `pub` 是为了让 `lib.rs` 的
/// stronghold plugin Builder 复用同一份 hash 逻辑；该 plugin 已移除，
/// 现在仅 `crypto::init` 内部使用。
pub(crate) fn derive_stronghold_key(passphrase: &[u8]) -> [u8; STRONGHOLD_KEY_LEN] {
    type Blake2b256 = Blake2b<blake2::digest::consts::U32>;
    let mut hasher = Blake2b256::new();
    hasher.update(passphrase);
    let out = hasher.finalize();
    let mut key = [0u8; STRONGHOLD_KEY_LEN];
    key.copy_from_slice(&out);
    key
}

/// 取出 CryptoState 引用，避免对 Arc 的额外 clone + 简化调用方
fn crypto(app: &AppHandle) -> tauri::State<'_, CryptoState> {
    app.state::<CryptoState>()
}

/// 初始化 stronghold：放 `app.manage(CryptoState)`
///
/// 幂等：重复调用仅返回现有实例。
///
/// ## 关键修复（P0）：启动恢复语义
///
/// `Stronghold::new` 只把 snapshot 数据加载到 `self.snapshot`，**不会**自动把
/// client 恢复到 `self.clients` HashMap。若直接用 `get_client + create_client + save`
/// 的模式：
///   1. `get_client("icepaw")` 查 `self.clients` 找不到 → 返回 `ClientDataNotPresent`
///   2. 误判为"首次启动"，调用 `create_client("icepaw")` → 在 `self.clients`
///      里插入一个**空的** client（不会覆盖 self.snapshot 已有的 keystore/vault/store）
///   3. `save()` 走 `commit_with_keyprovider` —— 遍历 `self.clients` 把当前
///      （含空 client 的）状态写回 snapshot 文件 → **用空 client 覆盖了原本
///      已存在的 keystore/vault/store** → 重启后所有 agent 的 api_key 丢失。
///
/// 正确顺序：先 `load_client` 主动从 `self.snapshot` 恢复到 `self.clients`；
/// 仅当 `load_client` 返回 `ClientDataNotPresent`（snapshot 里确实没有该 client）
/// 时才走 `create_client + save` 首次创建。其他任何错误原样向上抛，不要 fallback。
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
        let guard: MutexGuard<'_, Stronghold> = sh_arc.lock().unwrap_or_else(|e| e.into_inner());

        // ★ P0 修复点：用 `load_client` 而不是 `get_client`。
        //   `load_client` 走 `self.snapshot → self.clients` 的恢复路径，
        //   `get_client` 只查 `self.clients`（在 `Stronghold::new` 之后永远为空）。
        match guard.load_client(CLIENT_NAME) {
            Ok(_) => {
                info!(
                    target: "ice_paw.crypto",
                    "Stronghold client 已从 snapshot 恢复（不需重新落盘）"
                );
                // 不调用 save()：从 snapshot 恢复 = 内存状态与磁盘一致，再 save
                // 是无意义的 I/O。
            }
            Err(e) => {
                // 区分"首次启动（snapshot 里没有 client）"和其他错误：
                //   - `ClientDataNotPresent`：`load_client` 在 snapshot 里查不到
                //     该 client_id，触发"首次创建"路径。
                //   - 其他错误（如 `CorruptedContent`、`SnapshotFileMissing`、
                //     `IllegalKeySize`、`ClientAlreadyLoaded` 等）：原样向上抛，
                //     绝不 fallback 到 create_client（避免再次覆盖 snapshot）。
                //
                // 注：用强类型 `match` 区分 ClientDataNotPresent 与其他错误，
                // 避免字符串匹配的脆弱性（`ClientError::ClientDataNotPresent` 的
                // Display 是 "error loading client data; no data present"，**不含**
                // 字面量 "ClientDataNotPresent"）。`iota_stronghold` 已在 Cargo.toml
                // 直接依赖。
                if matches!(e, ShClientError::ClientDataNotPresent) {
                    info!(
                        target: "ice_paw.crypto",
                        "Stronghold client 不存在，首次创建并落盘"
                    );
                    guard
                        .create_client(CLIENT_NAME)
                        .map_err(|e| AppError::Stronghold(format!("create_client: {e}")))?;
                    guard
                        .save()
                        .map_err(|e| AppError::Stronghold(format!("init save: {e}")))?;
                } else {
                    return Err(AppError::Stronghold(format!("load_client: {e}")));
                }
            }
        }
    } // guard dropped here

    app.manage(CryptoState { inner: sh_arc });
    Ok(())
}

/// 储存 (api_key, base_url) 到 vault
///
/// 若 `insert` 成功但后续 `save` 失败，会做 best-effort 回滚（删除刚插入的 key），
/// 避免幽灵密钥残留：调用方收到错误、认为操作失败，但密钥实际已写入快照。
pub fn store_api_key(
    app: &AppHandle,
    agent_id: &str,
    api_key: &str,
    base_url: Option<&str>,
) -> AppResult<()> {
    if agent_id.is_empty() {
        return Err(AppError::Validation("agent_id 不能为空".into()));
    }

    let state = crypto(app);
    let sh = state.lock_sh();

    let record = ApiKeyRecord {
        api_key: api_key.to_string(),
        base_url: base_url.map(String::from),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec(&record)?;

    // 在显式作用域内做 insert，让 store 引用在 save 前释放
    let insert_result = {
        let client = sh
            .get_client(CLIENT_NAME)
            .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
        let store = client.store();
        store.insert(agent_id.as_bytes().to_vec(), bytes, None)
    };
    insert_result.map_err(|e| AppError::Stronghold(format!("store.insert: {e}")))?;

    // 持久化；失败则 best-effort 回滚已插入的 key
    if let Err(e) = sh.save() {
        // 重新获取 store 做回滚（之前的 store 引用已随作用域释放）
        if let Ok(client) = sh.get_client(CLIENT_NAME) {
            let _ = client.store().delete(agent_id.as_bytes());
        }
        return Err(AppError::Stronghold(format!("store save: {e}")));
    }
    Ok(())
    // sh (guard) dropped here at function exit
}

/// 取回 (api_key, base_url)
pub fn fetch_api_key(app: &AppHandle, agent_id: &str) -> AppResult<(String, Option<String>)> {
    let state = crypto(app);
    let sh = state.lock_sh();

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

/// 删除该 agent 对应的密钥。
///
/// - 若密钥存在：删除并持久化
/// - 若密钥不存在（`store.delete` 返回 `Ok(None)`）：无操作，返回成功
/// - 存储层错误：向上传播（不再静默丢弃）
pub fn delete_api_key(app: &AppHandle, agent_id: &str) -> AppResult<()> {
    let state = crypto(app);
    let sh = state.lock_sh();
    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    // `store.delete` 对"key 不存在"返回 Ok(None)（不是 Err），
    // 因此 Err 一定是真实存储错误，必须向上传播。
    if let Err(e) = store.delete(agent_id.as_bytes()) {
        return Err(AppError::Stronghold(format!("store.delete: {e}")));
    }
    sh.save()
        .map_err(|e| AppError::Stronghold(format!("delete save: {e}")))?;
    Ok(())
}

/// 列出所有已存 agent_id
pub fn list_agent_ids(app: &AppHandle) -> AppResult<Vec<String>> {
    let state = crypto(app);
    let sh = state.lock_sh();
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
    let state = crypto(app);
    let sh = state.lock_sh();
    let client = sh
        .get_client(CLIENT_NAME)
        .map_err(|e| AppError::Stronghold(format!("get_client: {e}")))?;
    let store = client.store();
    store
        .contains_key(agent_id.as_bytes())
        .map_err(|e| AppError::Stronghold(format!("store.contains_key: {e}")))
}

// =========================================================================
// memory_store 加密（XChaCha20-Poly1305，REQ-CHAT-048）
// =========================================================================
//
// 提供:
//   - `encrypt_blob`:加密任意字节 → `[nonce 24B][ciphertext+tag]` BLOB
//   - `decrypt_blob`:反向：解 BLOB → 原文
//
// 加密参数:
//   - 算法: XChaCha20-Poly1305（与 Stronghold vault 内部一致）
//   - Key 派生: blake2b256(DEFAULT_PASSPHRASE || MEMORY_KEY_DOMAIN) → 32B
//     与 `derive_stronghold_key` 走同一套 blake2b256 派生，但拼接了不同的
//     domain 字符串，确保 memory_store 与 Stronghold vault 不共享 key
//   - Nonce: 每次 `encrypt_blob` 调用 OsRng 生成 24 字节随机值，前置到密文
//
// BLOB 布局:
// ```text
// [nonce: 24 bytes][ciphertext: N bytes][poly1305 tag: 16 bytes]
// ```
//   - 最小长度 = 24 + 16 = 40 字节（空明文）
//   - `decrypt_blob` 长度校验在调用 AEAD 之前完成，避免错误输入触发越界

/// memory_store 加密 key 的派生域常量
///
/// 与 Stronghold vault 完全隔离的 domain 字节串：两者共用
/// `DEFAULT_PASSPHRASE` 但拼接了不同的 domain，最终 blake2b256 输出
/// 32 字节 key 互不复用（避免 cross-domain 密钥复用风险）。
///
/// **不要**改成复用 `derive_stronghold_key` 的输出 —— 加密用途不同、
/// 攻击面不同，应保持 key 隔离。
pub(crate) const MEMORY_KEY_DOMAIN: &[u8] = b"ice-paw:memory-store";

/// XChaCha20-Poly1305 nonce 长度（字节）
const MEMORY_NONCE_LEN: usize = 24;
/// Poly1305 认证 tag 长度（字节）
const MEMORY_TAG_LEN: usize = 16;
/// XChaCha20-Poly1305 key 长度（字节）
const MEMORY_KEY_LEN: usize = 32;
/// BLOB 最小合法长度（nonce + tag）
const MEMORY_BLOB_MIN_LEN: usize = MEMORY_NONCE_LEN + MEMORY_TAG_LEN;

/// 把 `DEFAULT_PASSPHRASE || MEMORY_KEY_DOMAIN` 派生为 32 字节 XChaCha20 key
///
/// 与 `derive_stronghold_key` 同型（blake2b256 输出 32 字节），但拼接
/// 了 `MEMORY_KEY_DOMAIN` 后缀做 domain separation。
///
/// `pub(crate)` —— 仅 crate 内使用。
pub(crate) fn derive_memory_key() -> [u8; MEMORY_KEY_LEN] {
    type Blake2b256 = Blake2b<blake2::digest::consts::U32>;
    let mut hasher = Blake2b256::new();
    hasher.update(DEFAULT_PASSPHRASE.as_bytes());
    hasher.update(MEMORY_KEY_DOMAIN);
    let out = hasher.finalize();
    let mut key = [0u8; MEMORY_KEY_LEN];
    key.copy_from_slice(&out);
    key
}

/// 加密任意字节序列，返回 `[nonce 24B][ciphertext+tag]` BLOB
///
/// ## 失败模式
///
/// 当前实现不会主动失败：
///   - key 由确定性 blake2b256 派生 → 总成功
///   - OsRng.fill_bytes 不会失败（OS 拒绝分配熵时 panic 而非 Result）
///   - XChaCha20-Poly1305 encrypt 仅在 plaintext 长度超 `core::u32::MAX`
///     时返回错误（约 4 GiB），业务场景不会触及
///
/// 为 API 一致性仍返回 `AppResult<Vec<u8>>`；未来若切换确定性 nonce /
/// 改用 user-supplied key 需在此返回 `AppError::Validation`。
///
/// ## 安全属性
///
/// - 每次调用都用 `OsRng` 生成新的 24 字节 nonce。XChaCha20-Poly1305
///   安全要求 nonce 不可复用；24 字节随机碰撞概率 ~2^-192，可视为唯一。
/// - Poly1305 tag 由 AEAD 框架自动追加到 ciphertext 末尾，16 字节固定。
/// - Empty plaintext 合法：返回 24 + 0 + 16 = 40 字节 BLOB。
pub fn encrypt_blob(plaintext: &[u8]) -> AppResult<Vec<u8>> {
    let key_bytes = derive_memory_key();
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));

    // 24 字节随机 nonce（每次调用独立）
    let mut nonce_bytes = [0u8; MEMORY_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    // AEAD encrypt：输出 = ciphertext || tag（16B）
    let ciphertext_with_tag = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::Validation(format!("memory_store 加密失败: {e}")))?;

    // 组装 BLOB：[nonce || ciphertext||tag]
    let mut blob = Vec::with_capacity(MEMORY_NONCE_LEN + ciphertext_with_tag.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext_with_tag);
    Ok(blob)
}

/// 解密 BLOB `[nonce 24B][ciphertext+tag]` → 原文
///
/// ## 失败模式
///
/// - BLOB 长度 < `MEMORY_BLOB_MIN_LEN`（40 字节）→ `AppError::Validation`，
///   消息包含"长度"（业务可读，便于上层定位"数据损坏 vs 长度不足"）。
/// - Poly1305 tag 校验失败（nonce 错位 / 密文被篡改 / 非本 key 加密 / empty
///   ciphertext 错配）→ `AppError::Validation`，消息包含"认证失败"。
///
/// **不**区分"长度不足"与"长度恰好但 tag 失败"两种情况下的调用方：
/// 测试 `decrypt_rejects_short_input` 同时覆盖两种情况，均应返 Validation。
pub fn decrypt_blob(blob: &[u8]) -> AppResult<Vec<u8>> {
    if blob.len() < MEMORY_BLOB_MIN_LEN {
        return Err(AppError::Validation(format!(
            "memory_store 密文长度不足（需至少 {} 字节，实际 {} 字节）",
            MEMORY_BLOB_MIN_LEN,
            blob.len()
        )));
    }

    let key_bytes = derive_memory_key();
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key_bytes));

    let (nonce_bytes, ciphertext_with_tag) = blob.split_at(MEMORY_NONCE_LEN);
    let nonce = XNonce::from_slice(nonce_bytes);

    cipher.decrypt(nonce, ciphertext_with_tag).map_err(|e| {
        AppError::Validation(format!(
            "memory_store 解密认证失败（密文被篡改 / nonce 不匹配 / 非本 key 加密）: {e}"
        ))
    })
}
