import { defineConfig } from "vite";

const DEV_PORT = 1420;

// Tauri expects a fixed port and reads Rust errors from the same terminal.
export default defineConfig({
  clearScreen: false,
  server: {
    port: DEV_PORT,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "safari15", outDir: "dist", emptyOutDir: true },
});
