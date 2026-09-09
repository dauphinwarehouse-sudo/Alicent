import { describe, expect, it } from "vitest";
import { redo, undo } from "prosemirror-history";
import { TextSelection } from "prosemirror-state";
import { createEditorState } from "./editor-state";
import {
  isExactSourcePreserved,
  parseMarkdown,
  richTextSchema,
  serializeMarkdown,
  supportWarnings,
} from "./schema";

const edgeCaseMarkdown =
  "---\r\ntitle: Тест\r\n---\r\n\r\n# Глава\r\n\r\nПривет, **мир** 👩🏽‍💻!  \r\nНовая строка.\r\n\r\n<!-- сохранить -->\r\n";

describe("lossless Markdown envelope", () => {
  it("round-trips Cyrillic, emoji, CRLF and unsupported syntax byte-for-byte", () => {
    const document = parseMarkdown(edgeCaseMarkdown);
    expect(serializeMarkdown(document)).toBe(edgeCaseMarkdown);
    expect(isExactSourcePreserved(document)).toBe(true);
  });

  it("survives block-schema JSON persistence", () => {
    const json = parseMarkdown(edgeCaseMarkdown).toJSON();
    const restored = richTextSchema.nodeFromJSON(json);
    expect(serializeMarkdown(restored)).toBe(edgeCaseMarkdown);
  });

  it("switches to canonical Markdown after a rich-text edit", () => {
    let state = createEditorState("# Заголовок\n\nТекст\n");
    const position = state.doc.content.size - 1;
    state = state.apply(state.tr.insertText(" ещё", position));
    expect(isExactSourcePreserved(state.doc)).toBe(false);
    expect(serializeMarkdown(state.doc)).toContain("Текст ещё");
  });

  it("restores the exact original Markdown after undo and redoes Cyrillic input", () => {
    const original = "# Заголовок\r\n\r\nТекст\r\n";
    let state = createEditorState(original);
    const dispatch = (transaction: Parameters<typeof state.apply>[0]) => {
      state = state.apply(transaction);
    };
    const position = state.doc.content.size - 1;
    state = state.apply(
      state.tr
        .setSelection(TextSelection.create(state.doc, position))
        .insertText(" по-русски"),
    );
    expect(serializeMarkdown(state.doc)).toContain("по-русски");
    expect(undo(state, dispatch)).toBe(true);
    expect(serializeMarkdown(state.doc)).toBe(original);
    expect(redo(state, dispatch)).toBe(true);
    expect(serializeMarkdown(state.doc)).toContain("по-русски");
  });

  it("flags constructs that must block automatic migration", () => {
    expect(supportWarnings(edgeCaseMarkdown)).toEqual(
      expect.arrayContaining([
        expect.stringContaining("front matter"),
        expect.stringContaining("HTML"),
      ]),
    );
  });
});
