// ESLint Flat Config (根级，覆盖 packages/ui 与 packages/app)
// 参考 IcePaw 组件库工程方案 §4.2：JS 推荐 + TS 推荐 + Vue 推荐 + Prettier 兼容
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import vue from "eslint-plugin-vue";
import prettier from "eslint-config-prettier/flat";

export default [
  // 基础推荐配置
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...vue.configs["flat/recommended"],

  // Vue 单文件组件：使用 TypeScript 解析器处理 <script setup lang="ts">
  // 同时开启浏览器全局（document / MouseEvent 等），避免 .vue 内 no-undef 误报
  {
    files: ["**/*.vue"],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
      },
      globals: {
        // 浏览器主线程常用全局
        document: "readonly",
        window: "readonly",
        navigator: "readonly",
        location: "readonly",
        history: "readonly",
        localStorage: "readonly",
        sessionStorage: "readonly",
        console: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        setInterval: "readonly",
        clearInterval: "readonly",
        requestAnimationFrame: "readonly",
        cancelAnimationFrame: "readonly",
        fetch: "readonly",
        URL: "readonly",
        URLSearchParams: "readonly",
        Blob: "readonly",
        File: "readonly",
        FileReader: "readonly",
        FormData: "readonly",
        Headers: "readonly",
        Request: "readonly",
        Response: "readonly",
        WebSocket: "readonly",
        Event: "readonly",
        CustomEvent: "readonly",
        EventTarget: "readonly",
        MouseEvent: "readonly",
        KeyboardEvent: "readonly",
        TouchEvent: "readonly",
        PointerEvent: "readonly",
        FocusEvent: "readonly",
        InputEvent: "readonly",
        WheelEvent: "readonly",
        DragEvent: "readonly",
        HTMLElement: "readonly",
        HTMLInputElement: "readonly",
        HTMLTextAreaElement: "readonly",
        HTMLButtonElement: "readonly",
        HTMLDivElement: "readonly",
        HTMLAnchorElement: "readonly",
        HTMLFormElement: "readonly",
        HTMLImageElement: "readonly",
        HTMLCanvasElement: "readonly",
        Element: "readonly",
        Node: "readonly",
        NodeList: "readonly",
        IntersectionObserver: "readonly",
        MutationObserver: "readonly",
        ResizeObserver: "readonly",
        PerformanceObserver: "readonly",
        AbortController: "readonly",
        AbortSignal: "readonly",
        TextEncoder: "readonly",
        TextDecoder: "readonly",
        crypto: "readonly",
      },
    },
  },

  // 全局忽略目录
  {
    ignores: [
      "dist/**",
      "**/dist/**",
      "src-tauri/**",
      "**/src-tauri/**",
      "node_modules/**",
      "**/node_modules/**",
      "src-tauri/gen/**",
      "**/gen/**",
      "target/**",
      "**/target/**",
      "packages/ui/dev/dist/**",
      "**/*.min.js",
      "**/*.bundle.js",
    ],
  },

  // 项目级规则覆盖
  {
    rules: {
      // IcePaw 采用单字组件名（如 Chat、Agent），关闭多字限制
      "vue/multi-word-component-names": "off",
      // Vue template 规则
      "vue/html-self-closing": ["error", { html: { void: "always", normal: "always", component: "always" } }],
      "vue/max-attributes-per-line": "off",
      "vue/attribute-hyphenation": ["error", "always"],
      "vue/v-on-event-hyphenation": ["error", "always"],
      // 允许下划线前缀参数/变量视为已使用
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
        },
      ],
    },
  },

  // 关闭与 Prettier 冲突的规则，必须放最后
  prettier,
];
