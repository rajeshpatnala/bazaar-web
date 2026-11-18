import { rmSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

/** The MSW worker script is for `dev:mock` only; never ship it in the production build. */
function dropMockWorker(): Plugin {
  let outDir = "dist";
  return {
    name: "drop-mock-worker",
    apply: "build",
    configResolved(config) {
      outDir = resolve(config.root, config.build.outDir);
    },
    closeBundle() {
      rmSync(resolve(outDir, "mockServiceWorker.js"), { force: true });
    },
  };
}

export default defineConfig({
  plugins: [react(), dropMockWorker()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": { target: "http://127.0.0.1:8787", changeOrigin: true, ws: true },
      "/auth": { target: "http://127.0.0.1:8787", changeOrigin: true },
    },
  },
});
