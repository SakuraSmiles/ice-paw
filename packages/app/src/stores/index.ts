// Pinia 根实例：创建并导出 pinia，供 main.ts 通过 app.use() 挂载
// 注意：pinia 必须先于任何调用 defineStore 的模块加载完成前被挂载，
// 但 store 文件本身可以在 pinia 挂载之后再被首次调用 —— 由 Pinia 自动注入。
import { createPinia } from "pinia";

// 创建全局唯一的 pinia 实例
const pinia = createPinia();

export default pinia;
