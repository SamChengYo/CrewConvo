import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// In development the API server runs on :8090; vite proxies /api (including the WebSocket).
export default defineConfig({
  plugins: [react()],
  server: {
    host: '0.0.0.0',
    port: 5173,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8090', ws: true },
    },
  },
})
