import path from "path";
import { defineConfig } from "vitest/config";

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
        publicGraphSync: path.resolve(__dirname, "src/publicGraphSync.ts"),
        ranking: path.resolve(__dirname, "src/ranking.ts"),
        privateEvents: path.resolve(__dirname, "src/privateEvents.ts"),
        privateContactFacts: path.resolve(__dirname, "src/privateContactFacts.ts"),
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
    ],
  },
});
