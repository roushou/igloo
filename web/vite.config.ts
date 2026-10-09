import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: {
    port: 3000,
    // The dev server proxies the API so the console stays same-origin, as when the server hosts it.
    proxy: { "/v1": "http://127.0.0.1:7000" },
  },
  plugins: [
    tailwindcss(),
    tanstackStart({ spa: { enabled: true, prerender: { outputPath: "/index.html" } } }),
    viteReact(),
  ],
  test: {
    environment: "happy-dom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
