import { redo, undo } from "prosemirror-history";
import { EditorView } from "prosemirror-view";
import { createEditorState } from "./editor-state";
import { makeLargeMarkdown } from "./large-document";
import {
  isExactSourcePreserved,
  serializeMarkdown,
  supportWarnings,
} from "./schema";
import "./style.css";

const sample = `# Глава 1\r\n\r\nПривет, **мир**! Это текст на русском.\r\n\r\n- Первый пункт\r\n- Второй пункт\r\n`;

function required<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing ${selector}`);
  return element;
}

const editorHost = required<HTMLDivElement>("#editor");
const source = required<HTMLTextAreaElement>("#source");
const fidelity = required<HTMLSpanElement>("#fidelity");
const warnings = required<HTMLUListElement>("#warnings");
const latency = required<HTMLOutputElement>("#latency");
const compositionState = required<HTMLSpanElement>("#composition-state");
function renderState(view: EditorView): void {
  const markdown = serializeMarkdown(view.state.doc);
  source.value = markdown;
  const exact = isExactSourcePreserved(view.state.doc);
  fidelity.textContent = exact ? "lossless: exact" : "normalised after edit";
  fidelity.dataset.mode = exact ? "exact" : "normalised";
  const messages = supportWarnings(markdown);
  warnings.replaceChildren(
    ...messages.map((message) => {
      const item = document.createElement("li");
      item.textContent = message;
      return item;
    }),
  );
}

const view = new EditorView(editorHost, {
  state: createEditorState(sample),
  dispatchTransaction(transaction) {
    view.updateState(view.state.apply(transaction));
    renderState(view);
  },
  attributes: {
    spellcheck: "true",
    "aria-label": "Rich-text документ",
  },
});
renderState(view);

required<HTMLButtonElement>("#undo").addEventListener("click", () => {
  undo(view.state, view.dispatch);
  view.focus();
});
required<HTMLButtonElement>("#redo").addEventListener("click", () => {
  redo(view.state, view.dispatch);
  view.focus();
});
required<HTMLButtonElement>("#load-source").addEventListener("click", () => {
  view.updateState(createEditorState(source.value));
  renderState(view);
  view.focus();
});

view.dom.addEventListener("compositionstart", () => {
  compositionState.textContent = "IME: composition active";
  compositionState.dataset.mode = "active";
});
view.dom.addEventListener("compositionend", () => {
  compositionState.textContent = "IME: committed";
  compositionState.dataset.mode = "exact";
});

required<HTMLButtonElement>("#large-doc").addEventListener(
  "click",
  async () => {
    latency.textContent = "Генерация и разбор 200k слов…";
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => resolve()),
    );
    const markdown = makeLargeMarkdown();
    const parseStart = performance.now();
    const state = createEditorState(markdown);
    const parseMs = performance.now() - parseStart;
    const renderStart = performance.now();
    view.updateState(state);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => resolve()),
    );
    const renderMs = performance.now() - renderStart;
    const serializeStart = performance.now();
    serializeMarkdown(view.state.doc);
    const serializeMs = performance.now() - serializeStart;
    renderState(view);
    latency.textContent = `200k слов · parse/state ${parseMs.toFixed(1)} ms · render frame ${renderMs.toFixed(1)} ms · serialize ${serializeMs.toFixed(1)} ms`;
  },
);
