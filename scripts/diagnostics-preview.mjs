import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

export const DIAGNOSTICS_SCHEMA_VERSION = "alicent.diagnostics-preview/v1";

const impacts = ["critical", "serious", "moderate", "minor"];

export function summarizeAxe(violations) {
  const summary = Object.fromEntries(impacts.map((impact) => [impact, 0]));
  for (const violation of violations) {
    if (impacts.includes(violation.impact)) summary[violation.impact] += 1;
  }
  return summary;
}

export function axeViolations(violations) {
  return violations.map(({ id, impact, help, helpUrl, nodes }) => ({
    id,
    impact: impact ?? null,
    help,
    helpUrl,
    nodes: nodes.length,
    targets: nodes.map((node) => node.target.map(String).join(" ")),
  }));
}

export async function writeDiagnosticsPreview(outputPath, preview) {
  if (preview.schemaVersion !== DIAGNOSTICS_SCHEMA_VERSION) {
    throw new Error(`Unsupported diagnostics schema: ${preview.schemaVersion}`);
  }
  await mkdir(path.dirname(outputPath), { recursive: true });
  await writeFile(outputPath, `${JSON.stringify(preview, null, 2)}\n`, "utf8");
}
