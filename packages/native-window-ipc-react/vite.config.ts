import { resolve } from "node:path";
import { defineConfig } from "vitest/config";
import dts from "vite-plugin-dts";

export default defineConfig({
  resolve: {
    alias: {
      "@fcannizzaro/native-window-ipc/client": resolve(
        __dirname,
        "../native-window-ipc/client.ts",
      ),
    },
  },
  test: {
    include: ["tests/**/*.test.{ts,tsx}"],
    environment: "jsdom",
  },
  build: {
    lib: {
      entry: {
        index: resolve(__dirname, "index.ts"),
      },
      formats: ["es", "cjs"],
    },
    outDir: "dist",
    minify: false,
    rollupOptions: {
      external: [
        "react",
        "react/jsx-runtime",
        "@fcannizzaro/native-window-ipc",
        "@fcannizzaro/native-window-ipc/client",
      ],
    },
  },
  plugins: [
    dts({ tsconfigPath: "./tsconfig.build.json" }) as any,
  ],
});
