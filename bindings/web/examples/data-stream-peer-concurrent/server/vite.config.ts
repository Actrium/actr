import path from 'path';
import { defineConfig } from 'vite';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
    plugins: [wasm(), topLevelAwait()],
    resolve: {
        alias: {
            '@actr/web': path.resolve(__dirname, '../../../packages/web-sdk/src'),
            '@actr/dom': path.resolve(__dirname, '../../../packages/actr-dom/src'),
        },
    },
    server: {
        host: true,
        port: 4176,
    },
    optimizeDeps: {
        exclude: ['@actr/web'],
    },
});
