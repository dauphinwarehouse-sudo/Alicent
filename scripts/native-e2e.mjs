import { chromium } from "playwright";
import AxeBuilder from "@axe-core/playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import {
  DIAGNOSTICS_SCHEMA_VERSION,
  axeViolations,
  summarizeAxe,
  writeDiagnosticsPreview,
} from "./diagnostics-preview.mjs";

function argument(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index === -1 ? fallback : process.argv[index + 1];
}

const endpoint = argument("endpoint", "http://127.0.0.1:9222");
const outputDirectory = path.resolve(
  argument("output", "test-results/native-windows"),
);
const installerSha256 = argument("installer-sha256", "unknown");
const executableName = argument("executable-name", "Alicent.exe");
const diagnosticsPath = path.join(outputDirectory, "diagnostics-preview.json");
const screenshotPath = path.join(outputDirectory, "installed-app.png");
const tags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function connectWithRetry() {
  const deadline = Date.now() + 90_000;
  let lastError;
  while (Date.now() < deadline) {
    try {
      return await chromium.connectOverCDP(endpoint);
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 1_000));
    }
  }
  throw new Error(`WebView2 CDP endpoint was not ready: ${lastError}`);
}

async function pageWithRetry(browser) {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    const page = browser
      .contexts()
      .flatMap((context) => context.pages())
      .find((candidate) => !candidate.url().startsWith("devtools:"));
    if (page) return page;
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error("The installed app did not expose a WebView page");
}

await mkdir(outputDirectory, { recursive: true });
let browser;
try {
  browser = await connectWithRetry();
  const page = await pageWithRetry(browser);

  await page.waitForLoadState("domcontentloaded");
  await page.getByRole("heading", { level: 1 }).waitFor({ timeout: 30_000 });
  const evidence = await page.evaluate(() => ({
    title: document.title,
    url: location.href,
    userAgent: navigator.userAgent,
    tauriBridge:
      typeof window.__TAURI_INTERNALS__ === "object" &&
      typeof window.__TAURI_INTERNALS__?.invoke === "function",
  }));
  const nativeOrigin =
    !/^https?:\/\/(127\.0\.0\.1|localhost):1420(?:\/|$)/.test(evidence.url) &&
    evidence.tauriBridge;

  const newProject = page.getByRole("button", {
    name: "Новый проект",
    exact: true,
  });
  await newProject.waitFor({ timeout: 30_000 });
  const projectActionEnabled = await newProject.isEnabled();
  const results = await new AxeBuilder({ page }).withTags(tags).analyze();
  const summary = summarizeAxe(results.violations);
  await page.screenshot({ path: screenshotPath, fullPage: true });

  const checks = [
    {
      id: "installed-native-boundary",
      status: nativeOrigin ? "passed" : "failed",
      details: nativeOrigin
        ? "Tauri bridge present and Vite development origin absent"
        : "Native Tauri evidence was not observed",
    },
    {
      id: "native-project-action",
      status: projectActionEnabled ? "passed" : "failed",
      details: projectActionEnabled
        ? "New project action is enabled in the installed application"
        : "New project action remained disabled as in browser preview mode",
    },
    {
      id: "wcag-critical",
      status: summary.critical === 0 ? "passed" : "failed",
      details: `${summary.critical} critical axe violation(s)`,
    },
  ];

  await writeDiagnosticsPreview(diagnosticsPath, {
    schemaVersion: DIAGNOSTICS_SCHEMA_VERSION,
    generatedAt: new Date().toISOString(),
    scope: {
      mode: "installed-native-windows",
      installedApplication: true,
      nativeBoundaryExercised: nativeOrigin,
    },
    environment: {
      os: "windows",
      runtime: "Installed Tauri application using WebView2",
      executableName,
      installerSha256,
      pageOrigin: new URL(evidence.url).origin,
      userAgent: evidence.userAgent,
    },
    checks,
    accessibility: {
      standard: "WCAG 2 A/AA and WCAG 2.1 A/AA axe tags",
      summary,
      violations: axeViolations(results.violations),
    },
    artifacts: [
      { kind: "screenshot", path: "native-windows/installed-app.png" },
    ],
  });

  if (checks.some((check) => check.status === "failed")) {
    throw new Error(
      `Native E2E checks failed: ${checks
        .filter((check) => check.status === "failed")
        .map((check) => check.id)
        .join(", ")}`,
    );
  }
} catch (error) {
  if (!browser) {
    await writeDiagnosticsPreview(diagnosticsPath, {
      schemaVersion: DIAGNOSTICS_SCHEMA_VERSION,
      generatedAt: new Date().toISOString(),
      scope: {
        mode: "installed-native-windows",
        installedApplication: true,
        nativeBoundaryExercised: false,
      },
      environment: {
        os: "windows",
        runtime: "Installed Tauri application using WebView2",
        executableName,
        installerSha256,
      },
      checks: [
        {
          id: "installed-native-boundary",
          status: "failed",
          details: String(error),
        },
      ],
      accessibility: {
        standard: "WCAG 2 A/AA and WCAG 2.1 A/AA axe tags",
        summary: { critical: 0, serious: 0, moderate: 0, minor: 0 },
        violations: [],
      },
      artifacts: [],
    });
  }
  console.error(error);
  process.exitCode = 1;
} finally {
  await browser?.close();
}
