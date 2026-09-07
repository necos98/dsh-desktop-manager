import { defineConfig } from "vitest/config";

// Porta fissa usata da Tauri (devUrl in tauri.conf.json).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Vite non deve osservare la cartella Rust (target/ in scrittura continua)
      ignored: ["**/src-tauri/**", "**/target/**", "**/node_modules/**"],
    },
  },
  build: {
    target: "es2022",
    outDir: "dist",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
