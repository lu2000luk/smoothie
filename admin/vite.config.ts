import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 5174,
    proxy: {
      "/api": {
        target: `http://localhost:${process.env.ADMIN_PORT || 4102}`,
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    chunkSizeWarningLimit: 900,
  },
});
