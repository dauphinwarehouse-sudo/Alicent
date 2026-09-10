import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Document, ProviderGenerationPort } from "@alicent/contracts";
import { AiPanel } from "./AiPanel";

const document: Document = {
  id: "scene-1",
  parent_id: null,
  title: "Разговор у ворот",
  kind: "scene",
  revision: 3,
  updated_at: "2026-09-10T00:00:00Z",
  content: "Он остановился у ворот.",
  ai_context_excluded: false,
  ai_context_pinned: false,
};

let originalShowModal: PropertyDescriptor | undefined;

function supportDialog() {
  originalShowModal = Object.getOwnPropertyDescriptor(
    HTMLDialogElement.prototype,
    "showModal",
  );
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value(this: HTMLDialogElement) {
      this.setAttribute("open", "");
    },
  });
}

afterEach(() => {
  cleanup();
  if (originalShowModal) {
    Object.defineProperty(
      HTMLDialogElement.prototype,
      "showModal",
      originalShowModal,
    );
  } else {
    delete (HTMLDialogElement.prototype as { showModal?: unknown }).showModal;
  }
  originalShowModal = undefined;
  vi.restoreAllMocks();
});

it("streams a proposal, shows its diff and applies only after confirmation", async () => {
  supportDialog();
  const user = userEvent.setup();
  const generated = "Он замер у запертых ворот.";
  const reference = {
    ...document,
    id: "scene-2",
    title: "Предыдущая сцена",
    content: "До ворот герои поссорились.",
    ai_context_pinned: true,
  };
  const loadContext = vi.fn(async () => [
    {
      documentId: reference.id,
      title: reference.title,
      content: reference.content,
    },
  ]);
  const port: ProviderGenerationPort = {
    generate: vi.fn(async (request, onEvent) => {
      expect(request.documentContent).toBe(document.content);
      expect(request.currentDocumentId).toBe(document.id);
      expect(request.contextDocuments).toEqual([
        {
          documentId: reference.id,
          title: reference.title,
          content: reference.content,
        },
      ]);
      onEvent({ type: "delta", text: "Он замер " });
      onEvent({ type: "delta", text: "у запертых ворот." });
      return {
        text: generated,
        inputTokens: 42,
        outputTokens: 9,
      };
    }),
    cancel: vi.fn(async () => undefined),
  };
  const onApply = vi.fn(async () => undefined);
  render(
    <AiPanel
      document={document}
      available
      port={port}
      contextOptions={[document, reference]}
      pinnedContextIds={[reference.id]}
      loadContext={loadContext}
      onApply={onApply}
    />,
  );

  await user.type(screen.getByLabelText("Задача"), "Добавь напряжение");
  expect(
    (
      screen.getByRole("checkbox", {
        name: /Предыдущая сцена/,
      }) as HTMLInputElement
    ).checked,
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Предложить редакцию" }));
  expect(loadContext).toHaveBeenCalledWith(["scene-2"]);
  expect(await screen.findByText("Предложение готово")).toBeTruthy();
  expect(screen.getByText("9 токенов")).toBeTruthy();

  await user.click(
    screen.getByRole("button", { name: "Сравнить и применить" }),
  );
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText(/остановился/)).toBeTruthy();
  expect(within(dialog).getByText(/замер/)).toBeTruthy();
  await user.click(
    within(dialog).getByRole("button", {
      name: "Применить как новую версию",
    }),
  );
  await waitFor(() =>
    expect(onApply).toHaveBeenCalledWith(generated, document.content),
  );
});

it("never offers excluded references and disables generation for an excluded target", () => {
  const excluded = {
    ...document,
    ai_context_excluded: true,
  };
  render(
    <AiPanel
      document={excluded}
      available
      contextOptions={[
        excluded,
        {
          ...document,
          id: "secret",
          title: "Секрет",
          ai_context_excluded: true,
        },
      ]}
      onApply={vi.fn()}
    />,
  );
  expect(screen.queryByRole("checkbox", { name: "Секрет" })).toBeNull();
  expect(screen.getByText(/Этот документ исключён из ИИ/)).toBeTruthy();
  expect(
    (
      screen.getByRole("button", {
        name: "Предложить редакцию",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
});

it("cancels the active native request without changing the document", async () => {
  const user = userEvent.setup();
  let rejectGeneration: ((reason: unknown) => void) | undefined;
  const port: ProviderGenerationPort = {
    generate: vi.fn(
      () =>
        new Promise<never>((_, reject) => {
          rejectGeneration = reject;
        }),
    ),
    cancel: vi.fn(async () => {
      rejectGeneration?.({ code: "ABORTED" });
    }),
  };
  render(
    <AiPanel document={document} available port={port} onApply={vi.fn()} />,
  );

  await user.type(screen.getByLabelText("Задача"), "Перепиши");
  await user.click(screen.getByRole("button", { name: "Предложить редакцию" }));
  await user.click(screen.getByRole("button", { name: "Остановить" }));
  await waitFor(() => expect(port.cancel).toHaveBeenCalled());
  expect(screen.queryByRole("alert")).toBeNull();
  expect(
    screen.getByRole("button", { name: "Предложить редакцию" }),
  ).toBeTruthy();
});
