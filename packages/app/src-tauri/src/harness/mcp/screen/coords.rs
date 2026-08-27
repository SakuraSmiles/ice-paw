//! computer use 坐标契约的纯数学 —— 不碰任何 Win32 API，全部无头可测。
//!
//! **坐标契约（docs/computer-use-roadmap.md 真相源）**：模型传的一切坐标
//! = 本会话**最近一次截图的图片像素空间**。截图可能被降采样（长边 ≤1600）、
//! 可能是局部裁剪、可能来自非主显示器，因此每次截图都要记一份
//! [`CaptureMeta`]（图片像素 ↔ 物理像素的换算上下文）：
//!
//! ```text
//! 图片像素 ──(img_to_phys: ×物理/发送比例)──▶ 物理像素（虚拟桌面绝对坐标）
//!                                              │
//! 物理像素 ──(phys_to_absolute: 0..=65535 归一化)──▶ SendInput 绝对坐标
//! ```
//!
//! 多显示器时虚拟桌面原点可为负（主显示器在右，左边显示器的左上角是
//! `(-2560, 0)` 这类坐标）——所有换算一律走 [`VirtualScreenLayout`] 基准，
//! 禁止假设原点为 0。

// =========================================================================
// 类型
// =========================================================================

/// 虚拟桌面布局（`GetSystemMetrics(SM_*VIRTUALSCREEN)` 快照）。
///
/// `origin` 是整个虚拟桌面左上角的**屏幕坐标**，多显示器时通常为负；
/// `width/height` 是虚拟桌面总尺寸。它同时是：
/// - 截图物理坐标的参考系（截图区域用它换算成绝对坐标）；
/// - SendInput `MOUSEEVENTF_VIRTUALDESK` 归一化的分母（布局变了 = 坐标过期）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualScreenLayout {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: i32,
    pub height: i32,
}

/// 物理像素矩形（虚拟桌面**绝对**坐标；区域捕获/显示器矩形共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysRect {
    /// 右边界（含）／下边界（含）之外的开区间终点，供钳制/相交判定用。
    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }
}

/// 一次截图的坐标映射元数据 —— 「图片像素空间 ↔ 物理像素空间」的全部换算上下文。
///
/// 每次成功截图写入 [`super::state::ScreenState`]（按会话键控），后续
/// `region` 裁剪与（操作阶段）鼠标键盘工具都从最近一份 meta 换算。
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureMeta {
    /// 截图时刻的虚拟桌面布局。输入前对比当前布局，不一致 = 坐标过期
    /// （用户插拔了显示器/改了分辨率），拒绝执行要求重新截图。
    pub layout: VirtualScreenLayout,
    /// 本次截图覆盖的物理区域（绝对坐标；裁剪截图 = 全屏/显示器矩形的子区域）。
    pub phys_region: PhysRect,
    /// 实际发给模型的图片尺寸（物理尺寸可能被降采样）。
    pub sent_width: u32,
    pub sent_height: u32,
    /// 捕获的显示器索引（None = 全虚拟桌面合并图）。
    pub monitor: Option<u32>,
}

// =========================================================================
// 换算
// =========================================================================

impl CaptureMeta {
    /// 图片像素 → 物理像素（虚拟桌面绝对坐标）。
    ///
    /// 比例 = 物理尺寸 / 发送尺寸（非整数倍时四舍五入），越界钳回区域——
    /// 模型给的坐标常见 ±1px 抖动，宁钳勿拒。
    pub fn img_to_phys(&self, ix: i64, iy: i64) -> (i32, i32) {
        let fx = self.phys_region.width as f64 / self.sent_width.max(1) as f64;
        let fy = self.phys_region.height as f64 / self.sent_height.max(1) as f64;
        let px = self.phys_region.x + (ix.max(0) as f64 * fx).round() as i32;
        let py = self.phys_region.y + (iy.max(0) as f64 * fy).round() as i32;
        let px = px.clamp(self.phys_region.x, self.phys_region.right() - 1);
        let py = py.clamp(self.phys_region.y, self.phys_region.bottom() - 1);
        (px, py)
    }

    /// 物理像素 → 图片像素（浮点，供摘要/调试反算；不承诺落在图内）。
    pub fn phys_to_img(&self, px: i32, py: i32) -> (f64, f64) {
        let fx = self.sent_width.max(1) as f64 / self.phys_region.width.max(1) as f64;
        let fy = self.sent_height.max(1) as f64 / self.phys_region.height.max(1) as f64;
        (
            (px - self.phys_region.x) as f64 * fx,
            (py - self.phys_region.y) as f64 * fy,
        )
    }
}

/// 物理像素 → SendInput 绝对坐标（`MOUSEEVENTF_ABSOLUTE|VIRTUALDESK`，0..=65535）。
///
/// VIRTUALDESK 以**整个虚拟桌面**为归一化域（单屏的 `SM_CXSCREEN` 归一化在
/// 多屏下会把坐标全挤进第一屏）。端点精确映射：
/// `origin → 0`、`origin + span - 1 → 65535`（等价经典 `MulDiv(v, 65535, span-1)`）。
/// 入参钳到布局内，返回值恒在 0..=65535。
pub fn phys_to_absolute(layout: &VirtualScreenLayout, px: i32, py: i32) -> (i32, i32) {
    let norm = |v: i32, origin: i32, span: i32| -> i32 {
        if span <= 1 {
            // 病态布局（1px 跨度）：归一化域退化为单点，任何钳后值都映到 0
            return 0;
        }
        let v = v.clamp(origin, origin + span - 1);
        ((v - origin) as i64 * 65535 / (span - 1) as i64) as i32
    };
    (
        norm(px, layout.origin_x, layout.width),
        norm(py, layout.origin_y, layout.height),
    )
}

/// 按长边上限计算发送尺寸（降采样专用；**不放大**——物理尺寸 ≤ max 时原样返回，
/// 至少 1px）。返回 (宽, 高)。
pub fn sent_size_for(phys_w: u32, phys_h: u32, max_long_side: u32) -> (u32, u32) {
    let long = phys_w.max(phys_h).max(1);
    let max = max_long_side.max(1);
    if long <= max {
        return (phys_w.max(1), phys_h.max(1));
    }
    let scale = max as f64 / long as f64;
    let w = ((phys_w as f64 * scale).round() as u32).max(1);
    let h = ((phys_h as f64 * scale).round() as u32).max(1);
    (w, h)
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 双显示器：左副屏 2560×1440（原点 -2560,0）+ 右主屏 2560×1440。
    fn dual_layout() -> VirtualScreenLayout {
        VirtualScreenLayout {
            origin_x: -2560,
            origin_y: 0,
            width: 5120,
            height: 1440,
        }
    }

    // ---------------------------------------------------------------------
    // phys_to_absolute
    // ---------------------------------------------------------------------

    #[test]
    fn absolute_endpoints_map_to_0_and_65535() {
        let l = dual_layout();
        // 虚拟桌面最左上（副屏左上角，负坐标）→ (0, 0)
        assert_eq!(phys_to_absolute(&l, -2560, 0), (0, 0));
        // 最右下 → (65535, 65535)
        assert_eq!(phys_to_absolute(&l, -2560 + 5119, 1439), (65535, 65535));
    }

    #[test]
    fn absolute_center_of_dual_screen_is_half() {
        // 两块等宽屏的接缝（物理 x=0）映到归一化域中点附近。像素 0 在虚拟桌面
        // 像素索引域（0..5119）里偏左半格，精确值 = 2560×65535/5119 ≈ 32773
        //（断言按同式计算，锁行为而非锁字面量；「中点附近」= 与 32767 差 ~6）。
        let l = dual_layout();
        let (x, _) = phys_to_absolute(&l, 0, 0);
        assert_eq!(x, 2560 * 65535 / 5119);
        assert!((x - 32767).abs() < 10, "应在归一化域中点附近，实际 {x}");
    }

    #[test]
    fn absolute_clamps_out_of_layout() {
        let l = dual_layout();
        // 远超布局的坐标钳到端点，不产生负数/超 65535
        assert_eq!(phys_to_absolute(&l, -99999, -99999), (0, 0));
        assert_eq!(phys_to_absolute(&l, 99999, 99999), (65535, 65535));
    }

    #[test]
    fn absolute_degenerate_single_pixel_layout() {
        // span=1（病态布局）：除零防御，钳制后恒 0
        let l = VirtualScreenLayout {
            origin_x: 0,
            origin_y: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(phys_to_absolute(&l, 0, 0), (0, 0));
    }

    // ---------------------------------------------------------------------
    // CaptureMeta::img_to_phys / phys_to_img
    // ---------------------------------------------------------------------

    #[test]
    fn img_to_phys_downscaled_center() {
        // 全虚拟桌面 5120×1440 降采样为 1600×450，取图中点 → 物理虚拟桌面中点
        let meta = CaptureMeta {
            layout: dual_layout(),
            phys_region: PhysRect {
                x: -2560,
                y: 0,
                width: 5120,
                height: 1440,
            },
            sent_width: 1600,
            sent_height: 450,
            monitor: None,
        };
        let (px, py) = meta.img_to_phys(800, 225);
        assert!(px.abs() <= 1, "物理 x 应在接缝 0 附近，实际 {px}");
        assert!((py - 720).abs() <= 2, "物理 y 应在 720 附近，实际 {py}");
    }

    #[test]
    fn img_to_phys_cropped_region_origin_is_region_not_screen() {
        // 裁剪截图：图片 (0,0) = 裁剪区域左上角（副屏中部），不是虚拟桌面原点
        let meta = CaptureMeta {
            layout: dual_layout(),
            phys_region: PhysRect {
                x: -2000,
                y: 400,
                width: 400,
                height: 300,
            },
            sent_width: 200,
            sent_height: 150,
            monitor: None,
        };
        assert_eq!(meta.img_to_phys(0, 0), (-2000, 400));
        // 右下角：2× 比例下图片 199 → 物理 398（索引映射，右缘物理像素 399
        // 与图片像素 199.5 对应——钳制上界内，不越界即可）
        let (px, py) = meta.img_to_phys(199, 149);
        assert_eq!(px, -2000 + 398);
        assert_eq!(py, 400 + 298);
    }

    #[test]
    fn img_to_phys_clamps_out_of_range() {
        let meta = CaptureMeta {
            layout: dual_layout(),
            phys_region: PhysRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            sent_width: 1280,
            sent_height: 720,
            monitor: Some(1),
        };
        // 模型越界坐标（负/超大）一律钳回区域，不 panic 不外溢
        assert_eq!(meta.img_to_phys(-50, -50), (0, 0));
        let (px, py) = meta.img_to_phys(999999, 999999);
        assert_eq!(px, 2559);
        assert_eq!(py, 1439);
    }

    #[test]
    fn img_phys_roundtrip_within_rounding() {
        let meta = CaptureMeta {
            layout: dual_layout(),
            phys_region: PhysRect {
                x: 100,
                y: 100,
                width: 2000,
                height: 1000,
            },
            sent_width: 1000,
            sent_height: 500,
            monitor: None,
        };
        // 图片 → 物理 → 图片，2×2 像素内的往返误差（比例非整除时四舍五入损失）
        for &(ix, iy) in &[(0u64, 0u64), (1, 1), (500, 250), (999, 499)] {
            let (px, py) = meta.img_to_phys(ix as i64, iy as i64);
            let (bx, by) = meta.phys_to_img(px, py);
            assert!(
                (bx - ix as f64).abs() <= 2.0 && (by - iy as f64).abs() <= 2.0,
                "({ix},{iy}) 往返误差过大: 物理({px},{py}) → 图片({bx:.1},{by:.1})"
            );
        }
    }

    // ---------------------------------------------------------------------
    // sent_size_for
    // ---------------------------------------------------------------------

    #[test]
    fn sent_size_landscape_and_portrait() {
        // 横屏 2560×1440 → 长边压 1600
        assert_eq!(sent_size_for(2560, 1440, 1600), (1600, 900));
        // 竖屏 1440×2560 → 长边是高
        assert_eq!(sent_size_for(1440, 2560, 1600), (900, 1600));
        // 已达标不放大（1:1 保留，OCR 精度优先）
        assert_eq!(sent_size_for(800, 600, 1600), (800, 600));
        assert_eq!(sent_size_for(1600, 900, 1600), (1600, 900));
    }

    #[test]
    fn sent_size_never_zero_and_extreme_ratio() {
        // 极长条（宽 100 高 10000）：高压 1600，宽按比例 ~2（≥1 兜底）
        let (w, h) = sent_size_for(100, 10000, 1600);
        assert_eq!(h, 1600);
        assert!((1..=20).contains(&w), "宽应在 1..=20，实际 {w}");
        // 病态 0 尺寸：max(1) 兜底不 panic
        assert_eq!(sent_size_for(0, 0, 1600), (1, 1));
    }
}
