import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  server: { port: 1420, strictPort: true },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (
            id.includes("@codemirror") ||
            id.includes("@lezer") ||
            id.includes("style-mod") ||
            id.includes("w3c-keyname")
          )
            return "editor";
          if (
            id.includes("node_modules/react") ||
            id.includes("node_modules/scheduler")
          )
            return "react";
        },
      },
    },
  },
  clearScreen: false,
});
