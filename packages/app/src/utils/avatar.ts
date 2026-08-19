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

/** 头像文件 → 压缩 dataURL（≤300KB 量级）。不是图片类型时抛错。 */
export async function compressAvatar(file: File): Promise<string> {
  if (!file.type.startsWith("image/")) {
    throw new Error("仅支持图片文件");
  }
  if (file.size > AVATAR_MAX_SRC_BYTES) {
    throw new AvatarTooLargeError(file.size);
  }
  const src = await readFile(file);
  const img = await loadImage(src);

  // 方形 cover 裁剪：取短边中心区域
  const edge = Math.min(img.naturalWidth, img.naturalHeight);
  const sx = (img.naturalWidth - edge) / 2;
  const sy = (img.naturalHeight - edge) / 2;
  // 不放大：源短边不足 MAX_EDGE 时按原尺寸输出
  const out = Math.min(edge, MAX_EDGE);

  const canvas = document.createElement("canvas");
  canvas.width = out;
  canvas.height = out;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 不可用");
  ctx.drawImage(img, sx, sy, edge, edge, 0, 0, out, out);

  // WebP 优先，回退 JPEG（个别环境 canvas.toDataURL('image/webp') 静默回退成 png
  // 兜底判断返回前缀）
  let data = canvas.toDataURL("image/webp", 0.85);
  if (!data.startsWith("data:image/webp")) {
    data = canvas.toDataURL("image/jpeg", 0.85);
  }
  return data;
}
