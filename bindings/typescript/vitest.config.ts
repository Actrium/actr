import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    coverage: {
      provider: 'v8',
      reporter: ['lcov', 'html'],
      reportsDirectory: './coverage',
      include: ['typescript/**/*.ts'],
    },
  },
});
