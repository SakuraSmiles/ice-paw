/**
 * avatar.ts — 头像源图大小上限（agent 上传管道；项目头像功能 2026-08-22 移除）
 *
 * 压缩/裁剪管道已整体迁往 AvatarCropper（vue-cropper：getCropBlob → png
 * dataURL，2026-08-21 成熟库优先拍板取代自研 canvas 裁剪）——本文件只剩
 * 源图上限常量（AvatarField 提前拦截 + AvatarCropper 内二次校验同源）。
 */

/** 原图上限：超过拒绝。防巨图解码压内存（产物 ≤256px 与源大小无关）；
 *  10MB 覆盖主流手机照片原图；裁剪产物才入库（≤~100KB），源图仅 objectURL 瞬时。 */
export const AVATAR_MAX_SRC_BYTES = 10 * 1024 * 1024;
