import fs from "fs";
import path from "path";
import { defineConfig } from "vitest/config";

// Tests that depend on the external hashtree repo (../../hashtree/) are
// excluded when that repo isn't checked out alongside this one.
const hashtreeAvailable = fs.existsSync(
  path.resolve(__dirname, "../../hashtree/ts/packages/hashtree/src/types.ts"),
);
const hashtreeTestExcludes = hashtreeAvailable
  ? []
  : [
      "tests/ProfileSearchIndex.test.ts",
      "tests/profileSearchIndexNhash.test.ts",
      "tests/publishProfileSearchIndex.test.ts",
    ];

export default defineConfig({
  root: __dirname,
  resolve: {
    alias: {
      "@msgpack/msgpack": path.resolve(
        __dirname,
        "../node_modules/@msgpack/msgpack/dist.esm/index.mjs",
      ),
    },
  },
  build: {
    rollupOptions: { external: ['nostr-tools'] },
    lib: {
      entry: {
        index: path.resolve(__dirname, "src/index.ts"),
        privateContactSyncV2: path.resolve(__dirname, "src/privateContactSyncV2.ts"),
        privateContactSyncV2Controller: path.resolve(__dirname, "src/privateContactSyncV2Controller.ts"),
        privateContactSync: path.resolve(__dirname, "src/privateContactSync.ts"),
        privateContactSyncController: path.resolve(__dirname, "src/privateContactSyncController.ts"),
      },
      name: "nostr-social-graph",
      formats: ["es", "cjs"],
      fileName: (format, entry) => entry === 'index'
        ? format === 'cjs' ? 'nostr-social-graph.cjs' : 'nostr-social-graph.es.js'
        : `${entry}.${format === 'cjs' ? 'cjs' : 'js'}`,
    },
    outDir: path.resolve(__dirname, "dist"),
  },
  test: {
    exclude: [
      "**/node_modules/**",
      "**/dist/**",
      "**/docs/**",
      "**/e2e/**",
      "**/.{idea,git,cache,output,temp}/**",
      ...hashtreeTestExcludes,
    ],
  },
});
