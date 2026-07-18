import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { viteStaticCopy } from "vite-plugin-static-copy";

export default defineConfig({
  plugins: [
    react(),
    viteStaticCopy({
      targets: [
        { src: "node_modules/cesium/Build/Cesium/Workers", dest: "cesium" },
        { src: "node_modules/cesium/Build/Cesium/ThirdParty", dest: "cesium" },
        { src: "node_modules/cesium/Build/Cesium/Assets", dest: "cesium" },
        { src: "node_modules/cesium/Build/Cesium/Widgets", dest: "cesium" }
      ]
    })
  ],
  define: {
    CESIUM_BASE_URL: JSON.stringify("/cesium")
  },
  server: {
    port: 5173,
    allowedHosts: ["host.docker.internal"],
    watch: {
      usePolling: process.env.CHOKIDAR_USEPOLLING === "true"
    },
    proxy: {
      "/v1": "http://localhost:8000",
      "/health": "http://localhost:8000"
    }
  }
});
