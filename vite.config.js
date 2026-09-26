import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import process from "node:process";
import sirv from "sirv";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
// Test media for the browser preview (dev-assets/, not in git): served by the
// dev server only, so it never ends up in the app bundle like static/ would.
const devAssets = {
  name: "geniusclip-dev-assets",
  configureServer(server) {
    server.middlewares.use(sirv("dev-assets", { dev: true }));
  },
};

export default defineConfig(() => ({
  plugins: [sveltekit(), devAssets],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
