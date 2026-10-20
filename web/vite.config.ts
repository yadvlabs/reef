import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { TanStackRouterVite } from '@tanstack/router-plugin/vite';

// The dev server proxies API and git traffic to a running `reef serve` on port 4920.
// VITE_BUILD_ID cache-busts the files under public/ that change without renaming
// (the logo): index.html and Logo.tsx append it as ?v=.
process.env.VITE_BUILD_ID ??= Date.now().toString(36);

export default defineConfig({
  plugins: [TanStackRouterVite({ target: 'react', autoCodeSplitting: true }), react()],
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
  server: {
    port: 5173,
    proxy: {
      '/api': { target: process.env.REEF_URL ?? 'http://localhost:4920', changeOrigin: false },
      '/login/device/code': { target: process.env.REEF_URL ?? 'http://localhost:4920' },
      '/login/oauth': { target: process.env.REEF_URL ?? 'http://localhost:4920' },
      '/avatars': { target: process.env.REEF_URL ?? 'http://localhost:4920' },
    },
  },
});
