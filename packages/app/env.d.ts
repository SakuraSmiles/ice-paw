/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  // 使用 Record<string, never> 表示空 props/bindings，unknown 替代 any 以获得更严格的类型
  const component: DefineComponent<Record<string, never>, Record<string, never>, unknown>;
  export default component;
}
