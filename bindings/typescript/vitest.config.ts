import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    exclude: [
      'actr-workload/**',
      'node_modules/**',
      '**/node_modules/**',
      '**/dist/**',
    ],
    coverage: {
      provider: 'v8',
      reporter: ['lcov', 'html'],
      reportsDirectory: './coverage',
      include: ['typescript/**/*.ts'],
    },
  },
});
