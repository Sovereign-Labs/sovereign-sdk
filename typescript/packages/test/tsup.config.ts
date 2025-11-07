import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/soak/index.ts"],
  outDir: "dist/soak",
  format: ["cjs", "esm"],
  splitting: false,
  sourcemap: true,
  clean: true,
  dts: true,
});
