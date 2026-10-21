import { defineConfig } from "vite";
import { existsSync, unlinkSync } from "node:fs";
import { resolve } from "node:path";

export default defineConfig({
  plugins: process.env.CPF_PUBLIC_BUILD === "1" ? [{
    name: "exclude-development-shell",
    closeBundle() {
      const copiedArt = resolve(__dirname, "dist", "dsi-off.png");
      if (existsSync(copiedArt)) unlinkSync(copiedArt);
    },
  }] : [],
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: { target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13" },
});

