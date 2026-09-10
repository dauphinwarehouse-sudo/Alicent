import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  Document,
  ProjectPort,
  ProviderGenerationPort,
} from "@alicent/contracts";
import { App } from "./App";
// Component test double only. Real CodeMirror is exercised in browser smoke tests.
vi.mock("./Editor", () => ({
  Editor: ({
    initial,
    onChange,
  }: {
    initial: string;
    onChange: (s: string) => void;
  }) => (
    <textarea
      aria-label="Текст документа"
      defaultValue={initial}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
const doc: Document = {
  id: "a",
  title: "Первая глава",
  kind: "scene",
  parent_id: null,
  content: "Исходный текст",
  revision: 0,
  updated_at: "2026-01-01T00:00:00Z",
};
function port(): ProjectPort {
  return {
    backupProject: vi.fn(),
    restoreBackup: vi.fn(),
    cancelRecovery: vi.fn(),
    createCheckpoint: vi.fn(),
    checkpoints: vi.fn(async () => []),
    checkpointPreview: vi.fn(),
    restoreCheckpoint: vi.fn(),
    createProject: vi.fn(),
    openProject: vi.fn(async () => ({
      id: "p",
      title: "Тестовая рукопись",
      schema_version: 1,
      created_at: "2026-01-01T00:00:00Z",
    })),
    list: vi.fn(async () => [doc, { ...doc, id: "b", title: "Вторая глава" }]),
    createDocument: vi.fn(),
    renameDocument: vi.fn(async (id, title) => ({
      ...doc,
      id,
      title,
      revision: 1,
    })),
    duplicateDocument: vi.fn(async (id, title, parent) => ({
      ...doc,
      id: `${id}-copy`,
      title,
      parent_id: parent,
    })),
    archived: vi.fn(async () => []),
    archiveDocument: vi.fn(async (command) => ({
      command_id: command.command_id,
      document_id: command.document_id,
      affected_count: 1,
    })),
    restoreArchived: vi.fn(async (command) => ({
      command_id: command.command_id,
      document_id: command.document_id,
      affected_count: 1,
    })),
    moveDocument: vi.fn(async (command) => ({
      ...doc,
      id: command.document_id,
      parent_id: command.parent_id,
      revision: command.expected_revision + 1,
    })),
    read: vi.fn(async (id) => ({ ...doc, id })),
    save: vi.fn(async (cmd) => ({ ...doc, content: cmd.content, revision: 1 })),
    search: vi.fn(async () => []),
    versions: vi.fn(async () => []),
    versionContent: vi.fn(),
    restore: vi.fn(),
  };
}
it("browser preview is explicit and cannot silently persist projects", () => {
  render(<App available={false} />);
  expect(screen.getByText("Предпросмотр интерфейса")).toBeTruthy();
  expect(
    (screen.getByRole("button", { name: "Новый проект" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});
it("opens a project and document through its port", async () => {
  const user = userEvent.setup();
  const api = port();
  render(<App port={api} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(await screen.findByRole("button", { name: /Первая глава/ }));
  expect(
    ((await screen.findByLabelText("Текст документа")) as HTMLTextAreaElement)
      .value,
  ).toBe("Исходный текст");
});
it("failed save prevents document switching and retains the draft", async () => {
  const user = userEvent.setup();
  const api = port();
  api.save = vi.fn().mockRejectedValue("Конфликт версий");
  render(<App port={api} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(await screen.findByRole("button", { name: /Первая глава/ }));
  await user.type(
    await screen.findByLabelText("Текст документа"),
    " — новая строка",
  );
  await user.click(screen.getByRole("button", { name: /Вторая глава/ }));
  await waitFor(() => expect(api.save).toHaveBeenCalled());
  expect(api.read).toHaveBeenCalledTimes(1);
  expect(
    (screen.getByLabelText("Текст документа") as HTMLTextAreaElement).value,
  ).toContain("новая строка");
});
it("renames and duplicates the selected document", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() {
      this.setAttribute("open", "");
    },
  });
  const user = userEvent.setup();
  const api = port();
  render(<App port={api} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(await screen.findByRole("button", { name: /Первая глава/ }));

  await user.click(screen.getByRole("button", { name: "Переименовать" }));
  const input = screen.getByLabelText("Название");
  await user.clear(input);
  await user.type(input, "Пролог");
  await user.click(screen.getAllByRole("button", { name: "Переименовать" })[1]);
  await waitFor(() => expect(api.renameDocument).toHaveBeenCalled());

  await user.click(screen.getByRole("button", { name: "Дублировать" }));
  await waitFor(() =>
    expect(api.duplicateDocument).toHaveBeenCalledWith(
      "a",
      "Пролог — копия",
      null,
      expect.any(String),
    ),
  );
});
it("archives with an explicit no-delete warning and restores from the separate archive", async () => {
  vi.spyOn(window, "confirm").mockReturnValue(true);
  const user = userEvent.setup();
  const api = port();
  api.archived = vi.fn(async () => [
    {
      ...doc,
      archived_at: "2026-01-02T00:00:00Z",
      affected_count: 1,
    },
  ]);
  render(<App port={api} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(
    (await screen.findAllByRole("button", { name: "В архив" }))[0],
  );
  expect(window.confirm).toHaveBeenCalledWith(
    expect.stringContaining("не удаляются"),
  );
  await waitFor(() => expect(api.archiveDocument).toHaveBeenCalled());

  await user.click(screen.getByRole("button", { name: "Архив" }));
  await user.click(await screen.findByRole("button", { name: "Восстановить" }));
  await waitFor(() => expect(api.restoreArchived).toHaveBeenCalled());
});

it("moves a document to a chosen active folder with revision and command guards", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() {
      this.setAttribute("open", "");
    },
  });
  const user = userEvent.setup();
  const api = port();
  const folder = {
    ...doc,
    id: "folder",
    title: "Часть I",
    kind: "folder" as const,
    content: "",
  };
  api.list = vi.fn(async (parent) =>
    parent === null
      ? [doc, { ...doc, id: "b", title: "Вторая глава" }, folder]
      : [],
  );
  render(<App port={api} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(
    (await screen.findAllByRole("button", { name: "Переместить" }))[0],
  );
  await user.selectOptions(
    screen.getByLabelText("Новое расположение"),
    "folder",
  );
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Переместить",
    }),
  );
  await waitFor(() =>
    expect(api.moveDocument).toHaveBeenCalledWith({
      command_id: expect.any(String),
      document_id: "a",
      parent_id: "folder",
      expected_revision: 0,
    }),
  );
});

it("checkpoints the manuscript before applying a confirmed AI proposal", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() {
      this.setAttribute("open", "");
    },
  });
  const user = userEvent.setup();
  const api = port();
  api.read = vi.fn(async (id) =>
    id === "b"
      ? {
          ...doc,
          id: "b",
          title: "Вторая глава",
          content: "Контекст второй главы",
        }
      : doc,
  );
  const aiPort: ProviderGenerationPort = {
    generate: vi.fn(async (request) => {
      expect(request.contextDocuments).toEqual([
        {
          documentId: "b",
          title: "Вторая глава",
          content: "Контекст второй главы",
        },
      ]);
      return {
        text: "Новая редакция",
        inputTokens: 20,
        outputTokens: 4,
      };
    }),
    cancel: vi.fn(async () => undefined),
  };
  render(<App port={api} aiPort={aiPort} available />);
  await user.click(screen.getByRole("button", { name: "Открыть проект" }));
  await user.click(await screen.findByRole("button", { name: /Первая глава/ }));
  await user.click(screen.getByRole("tab", { name: "ИИ" }));
  await user.type(screen.getByLabelText("Задача"), "Усиль сцену");
  await user.click(screen.getByRole("checkbox", { name: "Вторая глава" }));
  await user.click(screen.getByRole("button", { name: "Предложить редакцию" }));
  await user.click(
    await screen.findByRole("button", { name: "Сравнить и применить" }),
  );
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Применить как новую версию",
    }),
  );

  await waitFor(() => expect(api.createCheckpoint).toHaveBeenCalled());
  expect(api.createCheckpoint).toHaveBeenCalledWith(
    expect.any(String),
    "Перед ИИ-правкой: Первая глава",
  );
  expect(api.save).toHaveBeenCalledWith({
    command_id: expect.any(String),
    document_id: "a",
    expected_revision: 0,
    content: "Новая редакция",
  });
  expect(
    vi.mocked(api.createCheckpoint).mock.invocationCallOrder[0],
  ).toBeLessThan(vi.mocked(api.save).mock.invocationCallOrder[0]);
});
