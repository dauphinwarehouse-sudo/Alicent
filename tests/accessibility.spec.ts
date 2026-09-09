import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import path from "node:path";
import {
  DIAGNOSTICS_SCHEMA_VERSION,
  axeViolations,
  summarizeAxe,
  writeDiagnosticsPreview,
} from "../scripts/diagnostics-preview.mjs";

const tags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

test("browser smoke: automated WCAG audit writes diagnostics preview", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 940 });
  await page.goto("/tests/fixture.html");
  await page
    .getByRole("button", { name: "Открыть проект", exact: true })
    .click();
  await page
    .getByRole("button", { name: "01. Северный ветер", exact: true })
    .click();

  const results = await new AxeBuilder({ page }).withTags(tags).analyze();
  const summary = summarizeAxe(results.violations);
  const outputDirectory = path.resolve("test-results/accessibility");
  const screenshotPath = path.join(outputDirectory, "browser-smoke.png");
  await page.screenshot({ path: screenshotPath, fullPage: true });
  await writeDiagnosticsPreview(
    path.join(outputDirectory, "browser-smoke.json"),
    {
      schemaVersion: DIAGNOSTICS_SCHEMA_VERSION,
      generatedAt: new Date().toISOString(),
      scope: {
        mode: "browser-smoke",
        installedApplication: false,
        nativeBoundaryExercised: false,
      },
      environment: {
        os: process.platform,
        runtime: "Playwright Chromium against the Vite test fixture",
      },
      checks: [
        {
          id: "wcag-critical",
          status: summary.critical === 0 ? "passed" : "failed",
          details: `${summary.critical} critical axe violation(s)`,
        },
      ],
      accessibility: {
        standard: "WCAG 2 A/AA and WCAG 2.1 A/AA axe tags",
        summary,
        violations: axeViolations(results.violations),
      },
      artifacts: [
        {
          kind: "screenshot",
          path: "accessibility/browser-smoke.png",
        },
      ],
    },
  );

  expect(
    results.violations.filter((violation) => violation.impact === "critical"),
  ).toEqual([]);
});
