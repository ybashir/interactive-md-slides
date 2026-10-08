import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { browserNotices } from '../../scripts/browser-notices.mjs'

export default defineConfig({
  plugins: [vue(), browserNotices()],
  worker: { plugins: () => [browserNotices(true)] },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: false,
      },
      '/slidev': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: false,
        ws: true,
      },
      '/_gateway': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: false,
      },
    },
  },
})
