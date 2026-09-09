import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const cwd = fileURLToPath(new URL("..", import.meta.url));
const vitest = fileURLToPath(
  new URL("../../../node_modules/vitest/vitest.mjs", import.meta.url),
);
const result = spawnSync(
  process.execPath,
  [vitest, "run", "src/latency.test.ts", "--reporter=verbose"],
  {
    cwd,
    env: { ...process.env, RICH_TEXT_BENCH_WORDS: "200000" },
    stdio: "inherit",
  },
);
process.exit(result.status ?? 1);
