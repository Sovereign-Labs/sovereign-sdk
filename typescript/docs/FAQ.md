## I am getting WASM related errors when trying to use NPM packages with various JS frameworks like NextJS

As you have noticed, `@sovereign-sdk/web3` utilizes Web Assembly for some of it's functionality (wallet related) and this requires special
handling in most JS frameworks. Below we list some common solutions.

### NextJS

**WASM file not found error**

Can manfiest as the below error at runtime:

> [Error: ENOENT: no such file or directory, open '/var/task/.next/server/static/wasm/ec4a013ab60ab944.wasm'] {
> errno: -2,
> code: 'ENOENT',
> syscall: 'open',
> path: '/var/task/.next/server/static/wasm/ec4a013ab60ab944.wasm',
> page: '/'
> }

**Fix**

We need to ensure the WASM files end up at a file path that NextJS requires them, this usually requires copying the `.wasm` files as part of your NextJS config.

Below is an example of a working config (`next.config.mjs`):

```js
import { copyFile, mkdir } from "fs/promises";
import { join } from "path";
import * as fs from "node:fs";

const nextConfig = {
  webpack(config, { isServer }) {
    // First we anble webassembly with webpack
    config.experiments = {
      ...config.experiments,
      asyncWebAssembly: true,
      layers: true,
    };

    // Setup a loader for `.wasm` files
    config.module.rules.push({
      test: /\.wasm$/,
      type: "webassembly/async",
    });

    // Creates a basic plugin that copies WASM files into the expected directory
    config.plugins.push(
      new (class {
        apply(compiler) {
          compiler.hooks.afterEmit.tapPromise(
            "CopyWasmPlugin",
            async (compilation) => {
              if (isServer) {
                const outputPath = compilation.outputOptions.path;
                const staticWasmDir = join(outputPath, "static", "wasm");
                await mkdir(staticWasmDir, { recursive: true }).catch(() => {});
                console.log("Output path:", outputPath);
                console.log("Static WASM dir:", staticWasmDir);
                const searchDirs = [
                  join(outputPath, "chunks"),
                  outputPath,
                  join(outputPath, "app"),
                  join(outputPath, "pages"),
                  join(outputPath, "..", "static"),
                ];
                for (const dir of searchDirs) {
                  if (fs.existsSync(dir)) {
                    console.log(`Searching for WASM files in: ${dir}`);
                    const wasmFiles = findWasmFiles(dir);
                    console.log(
                      `Found ${wasmFiles.length} WASM files in ${dir}`
                    );
                    for (const wasmFile of wasmFiles) {
                      const fileName = wasmFile.split("/").pop();
                      const destPath = join(staticWasmDir, fileName);
                      try {
                        await copyFile(wasmFile, destPath);
                        console.log(
                          `Successfully copied ${fileName} to ${destPath}`
                        );
                      } catch (error) {
                        console.warn(
                          `Warning copying WASM file ${fileName}:`,
                          error
                        );
                      }
                    }
                  } else {
                    console.log(`Directory doesn't exist: ${dir}`);
                  }
                }
              }
            }
          );
        }
      })()
    );
    return config;
  },
};

function findWasmFiles(dir) {
  const results = [];
  const files = fs.readdirSync(dir);
  for (const file of files) {
    const filePath = join(dir, file);
    const stat = fs.statSync(filePath);
    if (stat.isDirectory()) {
      results.push(...findWasmFiles(filePath));
    } else if (file.endsWith(".wasm")) {
      results.push(filePath);
    }
  }
  return results;
}

export default nextConfig;
```
