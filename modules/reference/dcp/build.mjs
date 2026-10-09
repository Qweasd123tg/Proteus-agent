import { build } from "esbuild";
import { readFile, mkdir, copyFile } from "node:fs/promises";
import { resolve } from "node:path";

const upstream = resolve("node_modules/@tarquinen/opencode-dcp/lib");
await mkdir("dist", { recursive: true });
await copyFile("node_modules/@tarquinen/opencode-dcp/LICENSE", "dist/LICENSE.upstream");
await build({
  entryPoints: ["src/worker.mjs", "src/module.mjs"], outdir: "dist", bundle: true,
  platform: "node", format: "esm", target: "node22",
  external: ["@anthropic-ai/tokenizer"],
  banner: { js: 'import { createRequire as proteusRequire } from "node:module"; const require = proteusRequire(import.meta.url);' },
  plugins: [{
    name: "proteus-platform",
    setup(api) {
      api.onResolve({ filter: /(?:persistence|logger|notification|store)$/ }, ({ path, resolveDir }) => {
        const absolute = resolve(resolveDir, `${path}.ts`);
        const adapters = {
          [resolve(upstream, "state/persistence.ts")]: "persistence",
          [resolve(upstream, "logger.ts")]: "logger",
          [resolve(upstream, "ui/notification.ts")]: "notification",
          [resolve(upstream, "prompts/store.ts")]: "prompts",
        };
        const adapter = adapters[absolute];
        if (adapter) return { path: resolve(`src/platform/${adapter}.mjs`) };
      });
      // Expose upstream defaults/merge without calling its OpenCode config loader.
      api.onLoad({ filter: /opencode-dcp\/lib\/config\.ts$/ }, async ({ path }) => ({
        contents: `${await readFile(path, "utf8")}\nexport { defaultConfig, mergeLayer };`, loader: "ts",
      }));
    },
  }],
});
