import { copyFileSync, mkdirSync } from "node:fs";
mkdirSync("dist", { recursive: true });
const r = await Bun.build({ entrypoints: ["src/main.tsx"], outdir: "dist", minify: true, target: "browser", naming: "[name].js" });
if (!r.success) { console.error(r.logs.join("\n")); process.exit(1); }
copyFileSync("src/styles.css", "dist/styles.css");
copyFileSync("index.html", "dist/index.html");
copyFileSync("favicon.png", "dist/favicon.png");
