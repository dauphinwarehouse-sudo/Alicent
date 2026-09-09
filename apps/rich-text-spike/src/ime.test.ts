import { afterEach, describe, expect, it } from "vitest";
import { EditorView } from "prosemirror-view";
import { createEditorState } from "./editor-state";
import { serializeMarkdown } from "./schema";

let view: EditorView | undefined;
afterEach(() => {
  view?.destroy();
  view = undefined;
  document.body.replaceChildren();
});

describe("IME-friendly input boundary", () => {
  it("does not cancel native composition events and preserves composed Cyrillic", () => {
    const host = document.createElement("div");
    document.body.append(host);
    view = new EditorView(host, { state: createEditorState("Текст\n") });

    const start = new CompositionEvent("compositionstart", {
      bubbles: true,
      cancelable: true,
      data: "ж",
    });
    expect(view.dom.dispatchEvent(start)).toBe(true);
    expect(start.defaultPrevented).toBe(false);

    const beforeInput = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertCompositionText",
      data: "ж",
      isComposing: true,
    });
    expect(view.dom.dispatchEvent(beforeInput)).toBe(true);
    expect(beforeInput.defaultPrevented).toBe(false);

    view.dispatch(
      view.state.tr.insertText("ж", view.state.doc.content.size - 1),
    );
    view.dom.dispatchEvent(
      new CompositionEvent("compositionend", { bubbles: true, data: "ж" }),
    );
    expect(serializeMarkdown(view.state.doc)).toContain("Текстж");
  });
});
