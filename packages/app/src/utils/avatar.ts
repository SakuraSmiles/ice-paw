/**
 * avatar.ts — 头像图片压缩（上传管道共用：agent / project）
 *
 * file → Image → canvas 方形 cover 裁剪（取短边中心）→ 缩到 ≤256px →
 * WebP dataURL（WebView2 = Chromium，WebP 编码可用；回退 JPEG 0.85）。
 * 头像只存 DB base64（不落盘），行体积靠压缩上限控制。
 */

/** 原图上限：超过拒绝（防止拿照片原图当头像压库） */
export const AVATAR_MAX_SRC_BYTES = 2 * 1024 * 1024;

/** 输出边长上限（方形，源图短边小于此值则不放大） */
const MAX_EDGE = 256;

export class AvatarTooLargeError extends Error {
  constructor(bytes: number) {
    super(`图片过大（${(bytes / 1024 / 1024).toFixed(1)}MB），请选择 2MB 以内的图片`);
    this.name = "AvatarTooLargeError";
  }
}

function readFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(r.result as string);
    r.onerror = () => reject(new Error("读取图片失败"));
    r.readAsDataURL(file);
  });
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("图片解码失败"));
    img.src = src;
  });
}

/** 裁剪视窗偏移（0~1 归一化，AvatarCropper 拖动定位产出）。
 *  center = {x:0.5, y:0.5} 等价旧版固定中心裁剪。 */
export interface CropOffset {
  x: number;
  y: number;
}

/** 头像文件 → 压缩 dataURL（≤300KB 量级）。不是图片类型时抛错。 */
export async function compressAvatar(
  file: File,
  offset?: CropOffset,
): Promise<string> {
  if (!file.type.startsWith("image/")) {
    throw new Error("仅支持图片文件");
  }
  if (file.size > AVATAR_MAX_SRC_BYTES) {
    throw new AvatarTooLargeError(file.size);
  }
  const src = await readFile(file);
  const img = await loadImage(src);

  // 方形 cover 裁剪：取短边，中心点由 offset 指定（默认几何中心）
  const edge = Math.min(img.naturalWidth, img.naturalHeight);
  const cx = Math.round((offset?.x ?? 0.5) * img.naturalWidth);
  const cy = Math.round((offset?.y ?? 0.5) * img.naturalHeight);
  const sx = Math.min(Math.max(cx - edge / 2, 0), img.naturalWidth - edge);
  const sy = Math.min(Math.max(cy - edge / 2, 0), img.naturalHeight - edge);
  // 不放大：源短边不足 MAX_EDGE 时按原尺寸输出
  const out = Math.min(edge, MAX_EDGE);

  const canvas = document.createElement("canvas");
  canvas.width = out;
  canvas.height = out;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 不可用");
  ctx.drawImage(img, sx, sy, edge, edge, 0, 0, out, out);

  return encodeCanvas(canvas, hasSourceAlpha(img));
}


/** 已加载图 + 偏移 → 压缩 dataURL（AvatarCropper 确认时用，避免重复读文件/解码）。
 *  与 compressAvatar 同管道：方形 cover + ≤256px + 按透明度选编码。 */
export async function compressAvatarImage(
  img: HTMLImageElement,
  offset?: CropOffset,
): Promise<string> {
  const edge = Math.min(img.naturalWidth, img.naturalHeight);
  const cx = Math.round((offset?.x ?? 0.5) * img.naturalWidth);
  const cy = Math.round((offset?.y ?? 0.5) * img.naturalHeight);
  const sx = Math.min(Math.max(cx - edge / 2, 0), img.naturalWidth - edge);
  const sy = Math.min(Math.max(cy - edge / 2, 0), img.naturalHeight - edge);
  const out = Math.min(edge, MAX_EDGE);
  const canvas = document.createElement("canvas");
  canvas.width = out;
  canvas.height = out;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 不可用");
  ctx.drawImage(img, sx, sy, edge, edge, 0, 0, out, out);
  return encodeCanvas(canvas, hasSourceAlpha(img));
}

/** 源图是否含透明像素（采样 5 点：四角+中心，任一 alpha<255 即判定）。
 *  WKWebView 的 WebP 编码可能丢 alpha、JPEG 格式级无 alpha——透明图必须走 PNG。 */
function hasSourceAlpha(img: HTMLImageElement): boolean {
  const c = document.createElement("canvas");
  const n = 8; // 8×8 采样网格（比 5 点更稳，成本忽略不计）
  c.width = n;
  c.height = n;
  const ctx = c.getContext("2d", { willReadFrequently: true });
  if (!ctx) return false; // 检测不了按无透明（保守走有损，行为同旧版）
  ctx.drawImage(img, 0, 0, n, n);
  try {
    const d = ctx.getImageData(0, 0, n, n).data;
    for (let i = 3; i < d.length; i += 4) {
      if (d[i] < 255) return true;
    }
  } catch {
    return false;
  }
  return false;
}

/** 编码策略（2026-08-21 透明修复）：
 *  含 alpha → PNG（无损保透明；≤256px 头像体积可控）
 *  无 alpha → WebP 0.85 → JPEG 0.85 回退（体积优先） */
function encodeCanvas(canvas: HTMLCanvasElement, withAlpha: boolean): string {
  if (withAlpha) {
    return canvas.toDataURL("image/png");
  }
  let data = canvas.toDataURL("image/webp", 0.85);
  if (!data.startsWith("data:image/webp")) {
    data = canvas.toDataURL("image/jpeg", 0.85);
  }
  return data;
}
