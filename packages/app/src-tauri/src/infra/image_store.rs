//! 图片外置存储（U2-1 DB 膨胀治理）
//!
//! 背景：`messages.content_blocks` 里图片块以内联 base64 存储——生产库 613MB/703MB
//! 都是 base64 图片（UE5 截图单张可达 9MB，767 条消息含图）。本模块把图片字节
//! 外置到 `<app_data_dir>/images/<content-hash>.<ext>` 文件，DB 只留 `image_file`
//! 指针，读侧水合回内联 base64——DB 瘦身、字节无损、内容寻址去重。
//!
//! 形态约定（**JSON 字符串级变换**，不动 `ContentBlock` 枚举——约 20 处穷举匹配
//! 会因加字段而全量翻车）：
//! - 内联（写入前 / 水合后）: `{"type":"image","data":"<base64>","media_type":"image/png"}`
//! - 外置（落库后）: `{"type":"image","data":"","media_type":"image/png","image_file":"<hash>.png"}`
//!   （`data` 保留为空串而非删除——`ContentBlock::Image` 反序列化要求 `data` 字段在场，
//!   删除会让未水合的意外解析退化成整条消息空块 = 数据丢失；空串退化安全）
//! - 文件缺失（读侧降级）: `{"type":"text","text":"[图片内容已不可恢复]"}`
//!
//! 不变式：
//! - 写侧唯一入口 [`offload_json`]（接入 `repo::message::update_content_blocks` +
//!   `cleanup::finalize_assistant_message`），落库形态恒「外置」。
//! - 读侧唯一入口 [`hydrate_json`]（repo 全部返回 content_blocks 的读函数接入），
//!   进 `parse_content_blocks` / LLM / 对账平面前恒「内联」。
//! - [`images_dir`] 未初始化（测试 / boot 前）时两函数都是恒等 no-op——测试零感知，
//!   也保证 boot 早于目录初始化时不会误操作。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine as _;
use blake2::{Blake2b512, Digest};

/// 外置指针字段名（落库 JSON 的 image 块新增字段）。
pub const IMAGE_FILE_KEY: &str = "image_file";

/// 图片字节不可恢复时的降级占位文本（与 `harness::derive` 经 re-export 同源，
/// 单一真相源在此——infra 是 ContentBlock 协议层，derive 向上引用不反向依赖）。
pub const IMAGE_UNRECOVERABLE_MARKER: &str = "[图片内容已不可恢复]";

static IMAGES_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 初始化图片外置目录（`<data_dir>/images`）。boot 时调用一次（幂等，重复调用
/// 只补建目录、不覆盖已设路径）。
pub fn init(data_dir: &Path) {
    let dir = data_dir.join("images");
    let _ = std::fs::create_dir_all(&dir);
    let _ = IMAGES_DIR.set(dir);
}

/// 取已初始化的图片目录（未初始化 → None，调用方 no-op 降级）。
pub fn images_dir() -> Option<&'static Path> {
    IMAGES_DIR.get().map(|p| p.as_path())
}

/// `media_type` → 文件扩展名。水合读文件不依赖扩展名（字节从文件来、`media_type`
/// 从 JSON 字段来），扩展名纯调试 / OS 文件关联用，未知类型回落 `png` 无害。
fn ext_for_media_type(mt: &str) -> &'static str {
    match mt {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// 内容寻址：解码字节的 blake2b-512 前 16 字节 → 32 位 hex。
///
/// 比 kb 种子的「前 16 hex」（8 字节）多一倍——文件去重对碰撞更敏感（种子碰撞只
/// 是少更新一次，文件碰撞是两张不同图互相覆盖 = 数据损坏），16 字节 128 bit 的
/// 碰撞概率可忽略。
fn content_hash(bytes: &[u8]) -> String {
    let digest = Blake2b512::digest(bytes);
    let mut out = String::with_capacity(32);
    for &b in &digest[..16] {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 内容寻址去重写：已存在 = 去重命中 = 成功；否则写临时文件后原子 rename。
///
/// **为什么临时文件 + rename**：直接写目标路径在写一半崩溃/磁盘满时会留下半截
/// 文件，下次 sweep 见「已存在」误判去重命中 → 半截字节当全图 = 数据损坏。
/// 临时文件写坏只留下 `.tmp-*` 残片，目标路径永不半截；rename 是原子步。
async fn write_dedup(dir: &Path, filename: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = dir.join(filename);
    if tokio::fs::metadata(&path).await.is_ok() {
        return Ok(());
    }
    let tmp = dir.join(format!("{filename}.tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, bytes).await?;
    match tokio::fs::rename(&tmp, &path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            // Windows rename 遇目标已存在会失败——内容寻址下目标若已被并发写入，
            // 其字节与本次完全相同（同 hash），视为去重命中。
            if tokio::fs::metadata(&path).await.is_ok() {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

/// 写侧唯一入口：把 content_blocks JSON 里内联 base64 图片外置为文件。
///
/// 无图片 / 目录未初始化 / 坏 JSON / 坏 base64 / 文件写失败 → 对应块原样保留
/// （软失败：写失败保留内联，下次 sweep 重试，绝不因外置失败丢字节）。已外置
/// （`image_file` 在场）的块跳过（幂等，支持部分外置行的重试）。
pub async fn offload_json(json: &str) -> String {
    match images_dir() {
        Some(dir) => offload_json_into(json, dir).await,
        None => json.to_string(),
    }
}

async fn offload_json_into(json: &str, dir: &Path) -> String {
    // 快路径：无图片块 → 原样（绝大多数消息是纯文本）
    if !json.contains("\"image\"") {
        return json.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    let Some(arr) = value.as_array_mut() else {
        return json.to_string();
    };
    let mut changed = false;
    for block in arr.iter_mut() {
        let Some(obj) = block.as_object_mut() else { continue };
        if obj.get("type").and_then(|v| v.as_str()) != Some("image") {
            continue;
        }
        if obj.contains_key(IMAGE_FILE_KEY) {
            continue; // 已外置
        }
        let Some(data) = obj.get("data").and_then(|v| v.as_str()) else { continue };
        if data.is_empty() {
            continue; // 空图（剥离层已处理）不外置
        }
        let media_type = obj
            .get("media_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image/png");
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(data) else {
            continue; // 坏 base64 → 保留原样（读侧 parse 自会容错）
        };
        let filename = format!("{}.{}", content_hash(&decoded), ext_for_media_type(media_type));
        match write_dedup(dir, &filename, &decoded).await {
            Ok(()) => {
                obj.insert("data".to_string(), serde_json::Value::String(String::new()));
                obj.insert(
                    IMAGE_FILE_KEY.to_string(),
                    serde_json::Value::String(filename),
                );
                changed = true;
            }
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.image_store",
                    filename,
                    "图片外置写文件失败（保留内联，下次重试）: {e}"
                );
            }
        }
    }
    if changed {
        serde_json::to_string(&value).unwrap_or_else(|_| json.to_string())
    } else {
        json.to_string()
    }
}

/// 读侧唯一入口：把 content_blocks JSON 里 `image_file` 指针读回内联 base64。
///
/// 文件缺失 → 整块降级为 `Text(IMAGE_UNRECOVERABLE_MARKER)`（诚实标注不可恢复，
/// 不让 ref 静默消失）；无 `image_file` / 目录未初始化 → 原样（内联直过）。
pub async fn hydrate_json(json: &str) -> String {
    match images_dir() {
        Some(dir) => hydrate_json_from(json, dir).await,
        None => json.to_string(),
    }
}

async fn hydrate_json_from(json: &str, dir: &Path) -> String {
    if !json.contains(IMAGE_FILE_KEY) {
        return json.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    let Some(arr) = value.as_array_mut() else {
        return json.to_string();
    };
    let mut changed = false;
    for block in arr.iter_mut() {
        let Some(obj) = block.as_object_mut() else { continue };
        if obj.get("type").and_then(|v| v.as_str()) != Some("image") {
            continue;
        }
        let Some(filename) = obj.get(IMAGE_FILE_KEY).and_then(|v| v.as_str()) else {
            continue;
        };
        // 防路径逃逸：image_file 只可能是我们生成的 `<hex>.<ext>`；目录分隔符 /
        // 盘符 / 相对路径父级一律判坏，降级 marker（文件名不可信 = 数据不可恢复）。
        if filename.is_empty()
            || filename.contains('/')
            || filename.contains('\\')
            || filename.contains("..")
        {
            *block = serde_json::json!({ "type": "text", "text": IMAGE_UNRECOVERABLE_MARKER });
            changed = true;
            continue;
        }
        let path = dir.join(filename);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                obj.insert("data".to_string(), serde_json::Value::String(encoded));
                obj.remove(IMAGE_FILE_KEY);
                changed = true;
            }
            Err(_) => {
                *block = serde_json::json!({ "type": "text", "text": IMAGE_UNRECOVERABLE_MARKER });
                changed = true;
            }
        }
    }
    if changed {
        serde_json::to_string(&value).unwrap_or_else(|_| json.to_string())
    } else {
        json.to_string()
    }
}

// =========================================================================
// 单元测试（用 `_into` / `_from` 显式传目录，绕过全局 OnceLock 防并行测试串扰）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("icepaw_imgstore_{}_{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn inline_image(data: &str, mt: &str) -> serde_json::Value {
        serde_json::json!({ "type": "image", "data": data, "media_type": mt })
    }

    #[tokio::test]
    async fn offload_hydrate_roundtrip_restores_exact_image() {
        let dir = tmp_dir("roundtrip");
        let bytes = vec![0xAAu8; 1024];
        let json = serde_json::to_string(&vec![
            serde_json::json!({ "type": "text", "text": "看图" }),
            inline_image(&b64(&bytes), "image/png"),
        ])
        .unwrap();

        let offloaded = offload_json_into(&json, &dir).await;
        // 落库形态：data 空 + image_file 在场，字节外置
        let v: serde_json::Value = serde_json::from_str(&offloaded).unwrap();
        let img = &v.as_array().unwrap()[1];
        assert_eq!(img["data"], "");
        assert!(img["image_file"].as_str().is_some());
        // 文件确实落盘
        let filename = img["image_file"].as_str().unwrap();
        assert!(dir.join(filename).exists());

        // 水合回内联：data 复原、image_file 摘除、media_type 保留
        let hydrated = hydrate_json_from(&offloaded, &dir).await;
        let v: serde_json::Value = serde_json::from_str(&hydrated).unwrap();
        let img = &v.as_array().unwrap()[1];
        assert_eq!(img["data"], b64(&bytes));
        assert!(img.get("image_file").is_none());
        assert_eq!(img["media_type"], "image/png");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn offload_is_idempotent_and_dedups() {
        let dir = tmp_dir("dedup");
        let bytes = vec![0x11u8; 2048];
        let a = serde_json::to_string(&vec![inline_image(&b64(&bytes), "image/jpeg")]).unwrap();
        let b = serde_json::to_string(&vec![inline_image(&b64(&bytes), "image/jpeg")]).unwrap();

        let off_a = offload_json_into(&a, &dir).await;
        let off_b = offload_json_into(&b, &dir).await;
        // 同字节 → 同文件名（去重）
        let fname_a = serde_json::from_str::<serde_json::Value>(&off_a).unwrap()[0]["image_file"]
            .as_str()
            .unwrap()
            .to_string();
        let fname_b = serde_json::from_str::<serde_json::Value>(&off_b).unwrap()[0]["image_file"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(fname_a, fname_b);
        // 目录里只有一份文件
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 1, "去重后只落一份: {files:?}");
        // 再 offload 一次 → 原样（幂等，无变化）
        assert_eq!(offload_json_into(&off_a, &dir).await, off_a);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hydrate_missing_file_degrades_to_marker() {
        let dir = tmp_dir("missing");
        // 直接构造一个 image_file 指向不存在的文件
        let json = serde_json::to_string(&vec![serde_json::json!({
            "type": "image", "data": "", "media_type": "image/png", "image_file": "deadbeef.png"
        })])
        .unwrap();
        let hydrated = hydrate_json_from(&json, &dir).await;
        let v: serde_json::Value = serde_json::from_str(&hydrated).unwrap();
        let block = &v.as_array().unwrap()[0];
        assert_eq!(block["type"], "text");
        assert_eq!(block["text"], IMAGE_UNRECOVERABLE_MARKER);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hydrate_rejects_path_traversal() {
        let dir = tmp_dir("traversal");
        let json = serde_json::to_string(&vec![serde_json::json!({
            "type": "image", "data": "", "media_type": "image/png", "image_file": "../../evil.png"
        })])
        .unwrap();
        let hydrated = hydrate_json_from(&json, &dir).await;
        let v: serde_json::Value = serde_json::from_str(&hydrated).unwrap();
        let block = &v.as_array().unwrap()[0];
        assert_eq!(block["type"], "text", "路径逃逸必须降级 marker");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn no_image_json_untouched() {
        let dir = tmp_dir("notext");
        let json = r#"[{"type":"text","text":"没有图"}]"#;
        assert_eq!(offload_json_into(json, &dir).await, json);
        assert_eq!(hydrate_json_from(json, &dir).await, json);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bad_json_untouched() {
        let dir = tmp_dir("badjson");
        let bad = "not json at all";
        assert_eq!(offload_json_into(bad, &dir).await, bad);
        assert_eq!(hydrate_json_from(bad, &dir).await, bad);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
