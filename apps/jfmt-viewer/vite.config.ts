import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  base: "./",
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: { port: 1420, strictPort: true },
});
