import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts", "src/wasm.ts"],
  format: ["cjs", "esm"],
  splitting: false,
  sourcemap: true,
  clean: true,
  dts: true,
});
