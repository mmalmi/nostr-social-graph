import { defineConfig } from 'vitest/config';

export default defineConfig({
  build: {
    lib: {
      entry: 'src/index.ts',
      name: 'nostr-social-graph',
      // The file name for the generated bundle (entry point of your library)
      fileName: (format) => `nostr-social-graph.${format}.js`,
    },
    outDir: 'dist',
  }
});
