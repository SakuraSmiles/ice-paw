import { defineConfig, mergeConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import { resolve } from "path";

export default mergeConfig(
  defineConfig({
    plugins: [vue()],
    resolve: {
      alias: {
        "@ice-paw/ui/styles": resolve(__dirname, "../ui/styles"),
      },
    },
  }),
  defineConfig({
    test: {
      environment: "happy-dom",
      globals: true,
      include: ["src/**/*.test.ts"],
      setupFiles: ["src/__tests__/setup.ts"],
    },
  }),
);
