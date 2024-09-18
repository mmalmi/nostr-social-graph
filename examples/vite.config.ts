import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  root: 'src',
  plugins: [react()],
  build: {
    rollupOptions: {
      input: 'index.html',
    },
    outDir: 'dist',
  },
});