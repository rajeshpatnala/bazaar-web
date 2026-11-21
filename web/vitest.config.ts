import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Force a non-IST browser zone so every time test proves IST conversion.
process.env.TZ = "America/New_York";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: false,
    css: false,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/api/schema.d.ts",
        "src/mocks/**",
        "src/test/**",
        "src/main.tsx",
        "src/vite-env.d.ts",
      ],
      thresholds: { lines: 75 },
    },
  },
});
