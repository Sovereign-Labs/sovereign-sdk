import { defineConfig } from "tsup";

export default defineConfig({
  entry: [
    "src/index.ts",
    "src/wasm.ts",
    "src/ledger-solana/default.ts",
    "src/ledger-solana/node.ts",
  ],
  format: ["cjs", "esm"],
  target: "es2020",
  splitting: false,
  sourcemap: true,
  clean: true,
  dts: true,
});
