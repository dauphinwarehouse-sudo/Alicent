import { expect, it } from "vitest";
import { createEditorState } from "./editor-state";
import { makeLargeMarkdown } from "./large-document";
import { serializeMarkdown } from "./schema";

it("keeps basic large-document parse/serialize latency observable", () => {
  const wordCount = 200_000;
  const markdown = makeLargeMarkdown(wordCount);
  const parseStart = performance.now();
  const state = createEditorState(markdown);
  const parseMs = performance.now() - parseStart;
  const serializeStart = performance.now();
  const roundTrip = serializeMarkdown(state.doc);
  const serializeMs = performance.now() - serializeStart;
  console.info(
    `[rich-text latency] ${wordCount} words: parse/state=${parseMs.toFixed(1)}ms serialize=${serializeMs.toFixed(1)}ms`,
  );
  expect(roundTrip).toBe(markdown);
  expect(parseMs).toBeLessThan(5_000);
  expect(serializeMs).toBeLessThan(5_000);
});
