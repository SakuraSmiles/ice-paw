//! PDF 页面 → PNG 渲染（Phase B 视觉路径，扫描件/图片型 PDF 专用）。
//!
//! 文本提取（`pdf_extract`）对扫描件/图片型 PDF 返回空——这种文档只能靠视觉模型读图。
//! 本模块用 pdfium（Chrome PDF 引擎）把指定页渲染成 PNG 字节，供 `view_attachment_image`
//! 工具作为 Image 块喂给视觉模型。
//!
//! ## pdfium 二进制
//! pdfium-render 是纯 Rust wrapper，**编译期不需要** pdfium 二进制，运行时动态加载。
//! 默认 feature `pdfium_6721` 必须搭配 bblanchon `chromium/6721` 版二进制（已下载到
//! `sodium-prebuilt/pdfium/bin/pdfium.dll`）。DLL 搜索顺序见 [`load_bindings`]。
//!
//! ## 线程模型（关键）
//! pdfium 非线程并发安全，且 `Pdfium` 是 `!Send`——不能跨线程共享，也不能进全局 Mutex。
//! 故采用**专用渲染线程**：首次调用时惰性 spawn 一个名为 `pdfium-render` 的 OS 线程，
//! 独占 Pdfium 实例；所有渲染请求经 `mpsc` 通道排队到该线程串行执行。调用方应把
//! `render_page_to_png` 包进 `spawn_blocking`（内部 `recv` 阻塞等结果）。

use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
// 注：mpsc::channel 返回 (Sender, Receiver)；reply 用普通 Sender（无界，单条结果够用）。

use image::ImageFormat;
use pdfium_render::prelude::*;

use crate::error::{AppError, AppResult};

/// 渲染目标宽度（像素）。PDF 多为矢量，固定宽度按页宽缩放，~1600px 在视觉模型可读
/// 与 token 成本间取衡（Anthropic 图片 token 随分辨率涨，1600px 单页约 1k token 出头）。
const RENDER_TARGET_WIDTH: i32 = 1600;

/// 渲染线程的作业。
enum Job {
    Render {
        bytes: Vec<u8>,
        page: usize,
        reply: Sender<Result<Vec<u8>, String>>,
    },
    Count {
        bytes: Vec<u8>,
        reply: Sender<Result<u16, String>>,
    },
}

/// 全局作业队列 sender（首次使用时惰性 spawn 渲染线程）。
static SENDER: OnceLock<Sender<Job>> = OnceLock::new();

fn sender() -> &'static Sender<Job> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("pdfium-render".into())
            .spawn(move || render_loop(rx))
            .expect("spawn pdfium-render 线程");
        tx
    })
}

/// 渲染线程主循环——独占 Pdfium，串行处理作业。
fn render_loop(rx: Receiver<Job>) {
    let pdfium = match load_bindings() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(target: "ice_paw.pdfium", error = %e, "pdfium 加载失败，渲染线程将排空作业后退出");
            while let Ok(job) = rx.recv() {
                let _ = reply(job, Err(format!("pdfium 加载失败: {e}")));
            }
            return;
        }
    };
    while let Ok(job) = rx.recv() {
        match job {
            Job::Render { bytes, page, reply } => {
                let r = render_impl(&pdfium, &bytes, page).map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            Job::Count { bytes, reply } => {
                let r = count_impl(&pdfium, &bytes).map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
        }
    }
}

/// 把 PDF 的第 `page`（1-based）页渲染成 PNG 字节。阻塞（等渲染线程），调用方请用
/// `spawn_blocking`。
pub fn render_page_to_png(pdf_bytes: &[u8], page: usize) -> AppResult<Vec<u8>> {
    let (rtx, rrx) = mpsc::channel();
    sender()
        .send(Job::Render {
            bytes: pdf_bytes.to_vec(),
            page,
            reply: rtx,
        })
        .map_err(|_| AppError::Internal("pdfium 渲染线程已退出".into()))?;
    rrx.recv()
        .map_err(|_| AppError::Internal("pdfium 渲染线程无响应".into()))?
        .map_err(AppError::Internal)
}

/// PDF 总页数（不渲染，仅解析页树）。阻塞，调用方请用 `spawn_blocking`。
pub fn page_count(pdf_bytes: &[u8]) -> AppResult<u16> {
    let (rtx, rrx) = mpsc::channel();
    sender()
        .send(Job::Count {
            bytes: pdf_bytes.to_vec(),
            reply: rtx,
        })
        .map_err(|_| AppError::Internal("pdfium 渲染线程已退出".into()))?;
    rrx.recv()
        .map_err(|_| AppError::Internal("pdfium 渲染线程无响应".into()))?
        .map_err(AppError::Internal)
}

/// 渲染实现（仅在渲染线程内调用，Pdfium 单线程独占）。
fn render_impl(pdfium: &Pdfium, pdf_bytes: &[u8], page: usize) -> AppResult<Vec<u8>> {
    let page_idx = page
        .checked_sub(1)
        .ok_or_else(|| AppError::Validation("页码必须 ≥ 1".into()))? as u16;
    let document = pdfium
        .load_pdf_from_byte_vec(pdf_bytes.to_vec(), None)
        .map_err(|e| AppError::Internal(format!("pdfium 打开 PDF 失败: {e}")))?;
    let pages = document.pages();
    let total = pages.len();
    if page_idx >= total {
        return Err(AppError::Validation(format!(
            "第 {page} 页不存在（共 {total} 页）"
        )));
    }
    let page_ref = pages
        .get(page_idx)
        .map_err(|e| AppError::Internal(format!("pdfium 取页失败: {e}")))?;
    let image = page_ref
        .render_with_config(&PdfRenderConfig::new().set_target_width(RENDER_TARGET_WIDTH))
        .map_err(|e| AppError::Internal(format!("pdfium 渲染失败: {e}")))?
        .as_image();
    let mut buf = Vec::with_capacity(256 * 1024);
    image
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| AppError::Internal(format!("PNG 编码失败: {e}")))?;
    Ok(buf)
}

fn count_impl(pdfium: &Pdfium, pdf_bytes: &[u8]) -> AppResult<u16> {
    let document = pdfium
        .load_pdf_from_byte_vec(pdf_bytes.to_vec(), None)
        .map_err(|e| AppError::Internal(format!("pdfium 打开 PDF 失败: {e}")))?;
    Ok(document.pages().len())
}

/// 把作业结果回传给通用的 reply（用于加载失败时排空任意作业）。
fn reply(job: Job, res: Result<Vec<u8>, String>) -> Result<(), ()> {
    match job {
        Job::Render { reply, .. } => reply.send(res).map_err(|_| ()),
        Job::Count { reply, .. } => {
            // count 期望 u16；加载失败时统一回 Err 字符串，类型无关。
            reply.send(res.map(|_| 0u16)).map_err(|_| ())
        }
    }
}

/// 按 DLL 搜索顺序加载 pdfium 绑定并构造 `Pdfium` 实例。
///
/// 顺序：
/// 1. `ICEPAW_PDFIUM_DIR` 环境变量指向的目录（显式覆盖，打包/测试用）。
/// 2. 可执行文件同目录（打包后 DLL 随 exe 放）。
/// 3. 开发回退：`sodium-prebuilt/pdfium/bin`（仓库内，本机 dev 用）。
/// 4. 系统库（PATH 里的 pdfium）。
fn load_bindings() -> AppResult<Pdfium> {
    let mut tried: Vec<String> = Vec::new();

    // 1. 环境变量
    if let Ok(dir) = std::env::var("ICEPAW_PDFIUM_DIR") {
        tried.push(format!("env ICEPAW_PDFIUM_DIR={dir}"));
        if let Ok(b) =
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
        {
            tracing::info!(target: "ice_paw.pdfium", dir = %dir, "已加载 pdfium（env）");
            return Ok(Pdfium::new(b));
        }
    }

    // 2. 可执行文件同目录
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            tried.push(format!("exe 目录 {}", dir.display()));
            if let Ok(b) = Pdfium::bind_to_library(
                Pdfium::pdfium_platform_library_name_at_path(dir),
            ) {
                tracing::info!(target: "ice_paw.pdfium", dir = %dir.display(), "已加载 pdfium（exe 目录）");
                return Ok(Pdfium::new(b));
            }
        }
    }

    // 3. 开发回退：仓库内预下载位置（CARGO_MANIFEST_DIR 在编译期烤进）
    let dev_dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../sodium-prebuilt/pdfium/bin"
    );
    tried.push(format!("dev 回退 {dev_dir}"));
    if let Ok(b) =
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dev_dir))
    {
        tracing::info!(target: "ice_paw.pdfium", dir = dev_dir, "已加载 pdfium（dev 回退）");
        return Ok(Pdfium::new(b));
    }

    // 4. 系统库
    tried.push("系统库 (PATH)".into());
    if let Ok(b) = Pdfium::bind_to_system_library() {
        tracing::info!(target: "ice_paw.pdfium", "已加载 pdfium（系统库）");
        return Ok(Pdfium::new(b));
    }

    Err(AppError::Internal(format!(
        "找不到 pdfium 二进制（搜索过：{}）。请把 pdfium.dll 放到可执行文件同目录，\
         或设 ICEPAW_PDFIUM_DIR 指向其目录。开发环境见 sodium-prebuilt/pdfium/bin/。",
        tried.join(" → ")
    )))
}
