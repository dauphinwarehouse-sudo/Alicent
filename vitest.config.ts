import { defineConfig } from 'vitest/config';
export default defineConfig({ test: { environment: 'jsdom', include: ['apps/**/*.test.{ts,tsx}','packages/**/*.test.ts'], restoreMocks: true }, esbuild: { jsx: 'automatic' } });
