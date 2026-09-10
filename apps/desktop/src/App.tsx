import { useEffect, useReducer, useRef, useState } from "react";
import { diffWords } from "diff";
import type {
  ArchivedDocument,
  Document,
  DocumentKind,
  DocumentSummary,
  Project,
  ProjectPort,
  ProviderContextDocument,
  ProviderGenerationPort,
  VersionSummary,
} from "@alicent/contracts";
import { api, desktopAvailable } from "./api";
import { AiPanel } from "./AiPanel";
import { Editor } from "./Editor";
import { EditorSession } from "./editor-session";
import { RecoveryDialog } from "./RecoveryDialog";

function Icon({ kind }: { kind: "search" | "history" }) {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      aria-hidden="true"
    >
      {kind === "search" ? (
        <>
          <circle cx="10" cy="10" r="6" />
          <path d="m15 15 5 5" />
        </>
      ) : (
        <>
          <path d="M4 8a8 8 0 1 1-1 7M4 3v5h5" />
          <path d="M12 7v6l3 2" />
        </>
      )}
    </svg>
  );
}
function NameDialog({
  title,
  initialValue = "",
  submitLabel = "Создать",
  onCancel,
  onSubmit,
}: {
  title: string;
  initialValue?: string;
  submitLabel?: string;
  onCancel: () => void;
  onSubmit: (name: string) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} onCancel={onCancel} aria-labelledby="dialog-title">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const name = String(
            new FormData(e.currentTarget).get("name") ?? "",
          ).trim();
          if (name) onSubmit(name);
        }}
      >
        <p className="eyebrow">ALICENT / СОЗДАНИЕ</p>
        <h2 id="dialog-title">{title}</h2>
        <label>
          Название
          <input
            name="name"
            required
            maxLength={200}
            autoFocus
            defaultValue={initialValue}
            placeholder="Например, Северный ветер"
          />
        </label>
        <div className="actions">
          <button type="button" onClick={onCancel}>
            Отмена
          </button>
          <button className="primary" type="submit">
            {submitLabel}
          </button>
        </div>
      </form>
    </dialog>
  );
}
type FolderDestination = {
  id: string;
  label: string;
  parentIds: string[];
};

class AiContextError extends Error {
  constructor(readonly code: string) {
    super(code);
  }
}

export async function loadMoveDestinations(
  port: ProjectPort,
  moving: DocumentSummary,
): Promise<FolderDestination[]> {
  const destinations: FolderDestination[] = [];
  const queue: Array<{
    parent: string | null;
    prefix: string;
    parentIds: string[];
  }> = [{ parent: null, prefix: "", parentIds: [] }];
  const visited = new Set<string>();
  while (queue.length) {
    const level = queue.shift();
    if (!level) break;
    for (let offset = 0; ; offset += 200) {
      const rows = await port.list(level.parent, offset);
      for (const row of rows) {
        if (row.kind !== "folder" || visited.has(row.id)) continue;
        visited.add(row.id);
        const insideMovingFolder =
          moving.kind === "folder" &&
          (row.id === moving.id || level.parentIds.includes(moving.id));
        if (insideMovingFolder) continue;
        const label = level.prefix
          ? `${level.prefix} / ${row.title}`
          : row.title;
        destinations.push({ id: row.id, label, parentIds: level.parentIds });
        queue.push({
          parent: row.id,
          prefix: label,
          parentIds: [...level.parentIds, row.id],
        });
      }
      if (rows.length < 200) break;
    }
  }
  return destinations;
}

function MoveDialog({
  document,
  destinations,
  onCancel,
  onSubmit,
}: {
  document: DocumentSummary;
  destinations: FolderDestination[];
  onCancel: () => void;
  onSubmit: (parentId: string | null) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const currentValue = document.parent_id ?? "__root__";
  const [value, setValue] = useState(currentValue);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} onCancel={onCancel} aria-labelledby="move-title">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (value !== currentValue)
            onSubmit(value === "__root__" ? null : value);
        }}
      >
        <p className="eyebrow">РУКОПИСЬ / ПЕРЕМЕЩЕНИЕ</p>
        <h2 id="move-title">Переместить «{document.title}»</h2>
        <p className="move-hint">
          Выберите папку назначения. Перемещение в корень доступно для сцен,
          заметок и папок.
        </p>
        <label>
          Новое расположение
          <select
            aria-label="Новое расположение"
            value={value}
            onChange={(event) => setValue(event.target.value)}
          >
            <option value="__root__">
              Корень{document.parent_id === null ? " (текущее)" : ""}
            </option>
            {destinations.map((destination) => (
              <option key={destination.id} value={destination.id}>
                {destination.label}
                {document.parent_id === destination.id ? " (текущее)" : ""}
              </option>
            ))}
          </select>
        </label>
        <div className="actions">
          <button type="button" onClick={onCancel}>
            Отмена
          </button>
          <button
            className="primary"
            type="submit"
            disabled={value === currentValue}
          >
            Переместить
          </button>
        </div>
      </form>
    </dialog>
  );
}

function RestoreDialog({
  revision,
  oldText,
  currentText,
  onCancel,
  onRestore,
}: {
  revision: number;
  oldText: string;
  currentText: string;
  onCancel: () => void;
  onRestore: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  const parts =
    oldText.length + currentText.length < 100_000
      ? diffWords(currentText, oldText, {
          intlSegmenter: new Intl.Segmenter("ru", { granularity: "word" }),
          timeout: 200,
          maxEditLength: 10_000,
        })
      : undefined;
  return (
    <dialog
      className="diff-dialog"
      ref={ref}
      onCancel={onCancel}
      aria-labelledby="restore-title"
    >
      <p className="eyebrow">ИСТОРИЯ / СРАВНЕНИЕ</p>
      <h2 id="restore-title">Восстановить версию {revision}?</h2>
      <p>
        Текущий текст останется в истории. Восстановление создаст новую версию.
      </p>
      <p className="diff-key">
        <del>Удаление</del> <ins>Добавление</ins>
      </p>
      <div className="diff-content">
        {parts ? (
          parts.map((part, i) =>
            part.added ? (
              <ins key={i}>{part.value}</ins>
            ) : part.removed ? (
              <del key={i}>{part.value}</del>
            ) : (
              <span key={i}>{part.value}</span>
            ),
          )
        ) : (
          <>
            <p>
              Лимит сравнения: показаны первые 10 000 символов выбранной версии
              без построчного сравнения.
            </p>
            <pre>{oldText.slice(0, 10_000)}</pre>
          </>
        )}
      </div>
      <div className="actions">
        <button onClick={onCancel}>Отмена</button>
        <button className="primary" onClick={onRestore}>
          Восстановить версию
        </button>
      </div>
    </dialog>
  );
}
export function App({
  port = api,
  available = desktopAvailable,
  aiPort,
}: {
  port?: ProjectPort;
  available?: boolean;
  aiPort?: ProviderGenerationPort;
}) {
  const [project, setProject] = useState<Project | null>(null);
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [archived, setArchived] = useState<ArchivedDocument[]>([]);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [folders, setFolders] = useState<{ id: string; title: string }[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const recoveryBusy = useRef(false);
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const [dialog, setDialog] = useState<"project" | DocumentKind | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [move, setMove] = useState<{
    document: DocumentSummary;
    destinations: FolderDestination[];
  } | null>(null);
  const [versions, setVersions] = useState<VersionSummary[]>([]);
  const [preview, setPreview] = useState<{
    revision: number;
    content: string;
  } | null>(null);
  const [focus, setFocus] = useState(false);
  const [inspectorTab, setInspectorTab] = useState<"ai" | "history">("history");
  const [editorEpoch, setEditorEpoch] = useState(0);
  const session = useRef<EditorSession | null>(null);
  const [, render] = useReducer((n) => n + 1, 0);
  const current = session.current;
  const parent = folders.at(-1)?.id ?? null;

  async function run(action: () => Promise<void>) {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(
        typeof e === "string"
          ? e
          : e instanceof Error
            ? e.message
            : "Операция не выполнена. Ваш текст остаётся в редакторе.",
      );
    } finally {
      running.current = false;
      setBusy(false);
    }
  }
  async function flush() {
    await session.current?.flush();
  }
  async function applyAiProposal(text: string, sourceContent: string) {
    if (running.current) throw new Error("Операция уже выполняется");
    running.current = true;
    setBusy(true);
    setError("");
    try {
      await flush();
      const active = session.current;
      if (!active || active.content !== sourceContent) {
        throw new Error(
          "Текст изменился после запроса. Правка не применена — запустите генерацию заново.",
        );
      }
      const source = active.document;
      await port.createCheckpoint(
        crypto.randomUUID(),
        `Перед ИИ-правкой: ${source.title}`.slice(0, 200),
      );
      const saved = await port.save({
        command_id: crypto.randomUUID(),
        document_id: source.id,
        expected_revision: source.revision,
        content: text,
      });
      setDocuments((rows) =>
        rows.map((row) => (row.id === saved.id ? saved : row)),
      );
      await select(saved);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "ИИ-правка не применена. Текущий текст остался без изменений.",
      );
      throw cause;
    } finally {
      running.current = false;
      setBusy(false);
    }
  }
  async function loadAiContext(
    ids: string[],
  ): Promise<ProviderContextDocument[]> {
    const uniqueIds = [...new Set(ids)];
    if (uniqueIds.length !== ids.length || uniqueIds.length > 8) {
      throw new AiContextError("CONTEXT_INVALID");
    }
    const activeId = session.current?.document.id;
    const allowed = new Set(
      documents
        .filter((document) => document.kind !== "folder")
        .map((document) => document.id),
    );
    const context: ProviderContextDocument[] = [];
    let totalBytes = 0;
    const encoder = new TextEncoder();
    for (const id of uniqueIds) {
      if (id === activeId || !allowed.has(id)) {
        throw new AiContextError("CONTEXT_STALE");
      }
      const document = await port.read(id);
      if (document.kind === "folder") {
        throw new AiContextError("CONTEXT_FOLDER");
      }
      totalBytes += encoder.encode(document.title).length;
      totalBytes += encoder.encode(document.content).length;
      if (totalBytes > 512 * 1024) {
        throw new AiContextError("CONTEXT_TOO_LARGE");
      }
      context.push({
        documentId: document.id,
        title: document.title,
        content: document.content,
      });
    }
    return context;
  }
  async function refresh(parentId = parent) {
    const rows = await port.list(parentId);
    setDocuments(rows);
    setHasMore(rows.length === 200);
  }
  async function refreshArchive() {
    setArchived(await port.archived());
  }
  async function archive(doc: DocumentSummary) {
    await flush();
    const warning =
      doc.kind === "folder"
        ? `Папка «${doc.title}» и всё её активное содержимое будут перемещены в архив одной операцией. Отдельно архивированные элементы останутся отдельными. Файлы не удаляются.`
        : `«${doc.title}» будет скрыт из рукописи и поиска. Файлы и история версий не удаляются.`;
    if (!window.confirm(warning)) return;
    await port.archiveDocument({
      command_id: crypto.randomUUID(),
      document_id: doc.id,
      expected_revision: doc.revision,
    });
    if (doc.kind === "folder" || session.current?.document.id === doc.id) {
      session.current?.dispose();
      session.current = null;
      render();
    }
    await refresh();
    await refreshArchive();
  }
  async function restoreArchived(doc: ArchivedDocument) {
    await port.restoreArchived({
      command_id: crypto.randomUUID(),
      document_id: doc.id,
      expected_revision: doc.revision,
    });
    await refreshArchive();
    await refresh();
  }
  async function select(doc: Document) {
    session.current?.dispose();
    session.current =
      doc.kind === "folder" ? null : new EditorSession(doc, port, render);
    setEditorEpoch((n) => n + 1);
    render();
    setPreview(null);
    setVersions([]);
    if (doc.kind !== "folder") setVersions(await port.versions(doc.id));
  }
  async function chooseProject(title?: string) {
    await flush();
    const next =
      title === undefined
        ? await port.openProject()
        : await port.createProject(title);
    if (!next) return;
    await activateProject(next);
  }
  async function activateProject(next: Project) {
    session.current?.dispose();
    session.current = null;
    setProject(next);
    setFolders([]);
    setQuery("");
    setSearching(false);
    setArchiveOpen(false);
    setArchived([]);
    setVersions([]);
    setDocuments([]);
    await refresh(null);
  }
  async function visit(doc: DocumentSummary) {
    await flush();
    if (doc.kind === "folder") {
      await refresh(doc.id);
      setFolders([...folders, { id: doc.id, title: doc.title }]);
      setSearching(false);
      setQuery("");
    } else {
      await select(await port.read(doc.id));
    }
  }
  async function openMove(doc: DocumentSummary) {
    await flush();
    const latest =
      session.current?.document.id === doc.id ? session.current.document : doc;
    setMove({
      document: latest,
      destinations: await loadMoveDestinations(port, latest),
    });
  }
  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (session.current?.dirty || recoveryBusy.current) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void session.current?.flush().catch(() => {});
      }
    };
    window.addEventListener("beforeunload", beforeUnload);
    window.addEventListener("keydown", keydown);
    let unlisten: (() => void) | undefined;
    let disposed = false;
    if (desktopAvailable) {
      void import("@tauri-apps/api/window")
        .then(async ({ getCurrentWindow }) => {
          const win = getCurrentWindow();
          const stop = await win.onCloseRequested(async (event) => {
            if (recoveryBusy.current) {
              event.preventDefault();
              setError(
                "Сначала дождитесь завершения операции сохранности или отмените её.",
              );
              return;
            }
            if (session.current?.dirty) {
              event.preventDefault();
              running.current = true;
              setBusy(true);
              try {
                await session.current.flush();
                await win.destroy();
              } catch {
                running.current = false;
                setBusy(false);
                setError(
                  "Окно не закрыто: сначала сохраните текст или устраните ошибку сохранения.",
                );
              }
            }
          });
          if (disposed) stop();
          else unlisten = stop;
        })
        .catch(() =>
          setError(
            "Защита закрытия окна недоступна. Сохраняйте текст перед выходом.",
          ),
        );
    }
    return () => {
      disposed = true;
      unlisten?.();
      session.current?.dispose();
      window.removeEventListener("beforeunload", beforeUnload);
      window.removeEventListener("keydown", keydown);
    };
  }, []);
  const labels = {
    saved: "Сохранено",
    dirty: "Есть изменения",
    saving: "Сохранение…",
    error: "Не сохранено",
  };
  const words = current?.content.trim()
    ? current.content.trim().split(/\s+/u).length
    : 0;
  return (
    <div className={`app ${focus ? "focus" : ""}`}>
      <div className="utilitybar">
        <span>Пространство автора</span>
        <span>Локально · без регистрации</span>
      </div>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            A
          </span>
          <div className="brand-title">
            <strong>Alicent</strong>
            <span className="brand-subtitle">Ваши истории — в ваших руках</span>
          </div>
          <span className="build-tag">Прототип 0.1</span>
        </div>
        <nav aria-label="Проект">
          <button
            disabled={!available || busy}
            onClick={() => setRecoveryOpen(true)}
          >
            Сохранность
          </button>
          <button
            disabled={!available || busy}
            onClick={() => setDialog("project")}
          >
            Новый проект
          </button>
          <button
            disabled={!available || busy}
            onClick={() => void run(() => chooseProject())}
          >
            Открыть проект
          </button>
        </nav>
      </header>
      <div className="sectionbar">
        <span className="current-section">Моя мастерская</span>
        {project && (
          <nav aria-label="Разделы мастерской">
            <a href="#project-tree" onClick={() => setFocus(false)}>
              Рукопись
            </a>
            <a href="#writing" onClick={() => setFocus(false)}>
              Редактор
            </a>
            <a
              href="#version-history"
              onClick={() => {
                setFocus(false);
                setInspectorTab("ai");
              }}
            >
              ИИ-соавтор
            </a>
            <a
              href="#version-history"
              onClick={() => {
                setFocus(false);
                setInspectorTab("history");
              }}
            >
              История
            </a>
          </nav>
        )}
        <span className="privacy-note">
          Тексты хранятся на вашем компьютере
        </span>
      </div>
      {error && (
        <div role="alert" className="error-banner">
          {error}
          <button onClick={() => setError("")} aria-label="Закрыть сообщение">
            ×
          </button>
        </div>
      )}
      {!project ? (
        <main className="welcome">
          <div className="welcome-copy">
            <p className="eyebrow">ДОБРО ПОЖАЛОВАТЬ В МАСТЕРСКУЮ</p>
            <h1>
              Здесь начинается
              <br />
              ваша история.
            </h1>
            <p className="lead">
              Собирайте главы, пишите без отвлечений и возвращайтесь к любой
              сохранённой версии. Произведение остаётся на вашем компьютере.
            </p>
            <div className="welcome-tags" aria-label="Возможности редактора">
              <span>Markdown</span>
              <span>Автосохранение</span>
              <span>История версий</span>
            </div>
            <div className="actions">
              <button
                className="primary"
                disabled={!available || busy}
                onClick={() => setDialog("project")}
              >
                Создать первый проект <span aria-hidden="true">↗</span>
              </button>
              <button
                disabled={!available || busy}
                onClick={() => void run(() => chooseProject())}
              >
                Открыть папку проекта
              </button>
            </div>
            {!available && (
              <div className="notice">
                <strong>Предпросмотр интерфейса</strong>
                <p>
                  Файлы проектов доступны только в Windows-приложении. Здесь
                  ничего не сохраняется и не имитируется. Для запуска из
                  исходников: <code>npm run desktop</code>.
                </p>
              </div>
            )}
          </div>
          <section
            className="welcome-detail"
            aria-label="Возможности прототипа"
          >
            <span className="section-number">ДЛЯ ВАШИХ ПРОИЗВЕДЕНИЙ</span>
            <h2>Всё важное — рядом</h2>
            <div className="feature">
              <span aria-hidden="true">▤</span>
              <div>
                <h3>Папки, сцены, заметки</h3>
                <p>Небольшие документы вместо одного огромного файла.</p>
              </div>
            </div>
            <div className="feature">
              <span aria-hidden="true">
                <Icon kind="history" />
              </span>
              <div>
                <h3>Сохранённые версии</h3>
                <p>Автосохранение, сравнение и восстановление версий.</p>
              </div>
            </div>
            <div className="feature">
              <span aria-hidden="true">
                <Icon kind="search" />
              </span>
              <div>
                <h3>Поиск по рукописи</h3>
                <p>Локальный полнотекстовый индекс. Интернет не нужен.</p>
              </div>
            </div>
            <p className="development-note">
              ИИ-редактор отправляет выбранную сцену только по вашему явному
              запросу и применяет предложение лишь после сравнения.
            </p>
          </section>
        </main>
      ) : (
        <div className="workspace" aria-busy={busy}>
          <aside className="sidebar" id="project-tree">
            <div className="project-heading">
              <p className="eyebrow">ПРОЕКТ</p>
              <h2 title={project.title}>{project.title}</h2>
            </div>
            <form
              className="search"
              onSubmit={(e) => {
                e.preventDefault();
                void run(async () => {
                  await flush();
                  setArchiveOpen(false);
                  if (!query.trim()) {
                    setSearching(false);
                    await refresh();
                  } else {
                    setDocuments(await port.search(query));
                    setSearching(true);
                    setHasMore(false);
                  }
                });
              }}
            >
              <label className="sr-only" htmlFor="search">
                Поиск по проекту
              </label>
              <input
                id="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Поиск по проекту"
                maxLength={2048}
              />
              <button type="submit" disabled={busy} aria-label="Найти">
                <Icon kind="search" />
              </button>
            </form>
            <div className="tree-heading">
              <h3>
                {archiveOpen
                  ? "Архив"
                  : searching
                    ? "Результаты поиска"
                    : "Рукопись"}
              </h3>
              <button
                aria-label="Новая сцена"
                disabled={busy || archiveOpen}
                onClick={() => setDialog("scene")}
              >
                +
              </button>
            </div>
            {!archiveOpen && (
              <div className="breadcrumbs">
                <button
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await refresh(null);
                      setFolders([]);
                      setSearching(false);
                      setQuery("");
                    })
                  }
                >
                  Корень
                </button>
                {folders.map((folder, i) => (
                  <button
                    key={folder.id}
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await refresh(folder.id);
                        setFolders(folders.slice(0, i + 1));
                        setSearching(false);
                        setQuery("");
                      })
                    }
                  >
                    / {folder.title}
                  </button>
                ))}
              </div>
            )}
            <nav className="document-list" aria-label="Документы">
              {archiveOpen
                ? archived.map((doc) => (
                    <div className="document-row archived-row" key={doc.id}>
                      <span title={doc.title}>
                        <span aria-hidden="true">
                          {doc.kind === "folder"
                            ? "▱"
                            : doc.kind === "note"
                              ? "◇"
                              : "▤"}
                        </span>{" "}
                        {doc.title}
                        <small>
                          {doc.affected_count} элем. ·{" "}
                          {new Date(doc.archived_at).toLocaleString("ru-RU")}
                        </small>
                      </span>
                      <button
                        disabled={busy}
                        onClick={() => void run(() => restoreArchived(doc))}
                      >
                        Восстановить
                      </button>
                    </div>
                  ))
                : documents.map((doc) => (
                    <div
                      className="document-row active-document-row"
                      key={doc.id}
                      role="group"
                      aria-label={doc.title}
                    >
                      <button
                        title={doc.title}
                        disabled={busy}
                        className={
                          current?.document.id === doc.id ? "selected" : ""
                        }
                        onClick={() => void run(() => visit(doc))}
                      >
                        <span aria-hidden="true">
                          {doc.kind === "folder"
                            ? "▱"
                            : doc.kind === "note"
                              ? "◇"
                              : "▤"}
                        </span>
                        <span>{doc.title}</span>
                      </button>
                      <button
                        className="move-document"
                        aria-label="Переместить"
                        title={`Переместить «${doc.title}»`}
                        disabled={busy}
                        onClick={() => void run(() => openMove(doc))}
                      >
                        ↗
                      </button>
                      <button
                        aria-label="В архив"
                        title={`Переместить «${doc.title}» в архив без удаления`}
                        disabled={busy}
                        onClick={() => void run(() => archive(doc))}
                      >
                        ⤓
                      </button>
                    </div>
                  ))}
              {(archiveOpen ? !archived.length : !documents.length) && (
                <p className="empty-small">
                  {archiveOpen
                    ? "Архив пуст."
                    : searching
                      ? "Совпадений нет. Попробуйте другое слово."
                      : "Здесь пока пусто. Создайте первую сцену."}
                </p>
              )}
              {!archiveOpen && hasMore && (
                <button
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      const more = await port.list(parent, documents.length);
                      setDocuments([...documents, ...more]);
                      setHasMore(more.length === 200);
                    })
                  }
                >
                  Загрузить ещё
                </button>
              )}
              {!archiveOpen && searching && documents.length === 200 && (
                <p className="empty-small">
                  Первые 200 совпадений. Уточните запрос.
                </p>
              )}
            </nav>
            <div className="tree-actions">
              <button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    if (archiveOpen) {
                      setArchiveOpen(false);
                      await refresh();
                    } else {
                      setArchiveOpen(true);
                      setSearching(false);
                      setQuery("");
                      await refreshArchive();
                    }
                  })
                }
              >
                {archiveOpen ? "← Рукопись" : "Архив"}
              </button>
              <button
                disabled={busy || archiveOpen}
                onClick={() => setDialog("folder")}
              >
                + Папка
              </button>
              <button
                disabled={busy || archiveOpen}
                onClick={() => setDialog("note")}
              >
                + Заметка
              </button>
            </div>
            <div className="local-note">
              <span aria-hidden="true">●</span> Локальный проект
              <span>Никакой облачной синхронизации</span>
            </div>
          </aside>
          <main className="writing-pane" id="writing">
            {current ? (
              <>
                <div className="document-toolbar">
                  <div>
                    <span className="eyebrow">
                      {current.document.kind === "note" ? "ЗАМЕТКА" : "СЦЕНА"} /
                      MARKDOWN
                    </span>
                    <h1>{current.document.title}</h1>
                  </div>
                  <div className="document-actions">
                    <button disabled={busy} onClick={() => setRenameOpen(true)}>
                      Переименовать
                    </button>
                    <button
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          await flush();
                          const source = session.current?.document;
                          if (!source) return;
                          const suffix = " — копия";
                          const title =
                            source.title.slice(0, 200 - suffix.length) + suffix;
                          const copy = await port.duplicateDocument(
                            source.id,
                            title,
                            source.parent_id,
                            crypto.randomUUID(),
                          );
                          await refresh(source.parent_id);
                          await select(copy);
                        })
                      }
                    >
                      Дублировать
                    </button>
                    <button
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          const source = session.current?.document;
                          if (source) await archive(source);
                        })
                      }
                    >
                      Архивировать
                    </button>
                    <button
                      onClick={() => setFocus(!focus)}
                      aria-pressed={focus}
                    >
                      {focus ? "Вернуть панели" : "Фокус"}
                    </button>
                  </div>
                </div>
                <div className="save-bar">
                  <span
                    role="status"
                    className={current.state === "error" ? "save-error" : ""}
                  >
                    {labels[current.state]}
                  </span>
                  <button
                    disabled={busy || current.state === "saving"}
                    onClick={() => void run(flush)}
                  >
                    Сохранить <kbd>Ctrl S</kbd>
                  </button>
                </div>
                {current.error && (
                  <div className="save-error-message" role="alert">
                    {current.error} При конфликте скопируйте черновик перед
                    повторным открытием проекта: автоматического перезаписывания
                    нет.
                  </div>
                )}
                <Editor
                  key={`${current.document.id}:${editorEpoch}`}
                  initial={current.content}
                  readOnly={busy || !!preview || !!dialog || recoveryOpen}
                  onChange={(text) => session.current?.edit(text)}
                />
                <footer className="editor-footer">
                  <span>
                    {words.toLocaleString("ru")} слов ·{" "}
                    {current.content.length.toLocaleString("ru")} симв.
                  </span>
                  <span>Версия {current.document.revision} · UTF-8</span>
                </footer>
              </>
            ) : (
              <div className="empty-editor">
                <p className="eyebrow">НАЧНИТЕ С ПЕРВОЙ СТРОКИ</p>
                <h1>
                  У каждой истории
                  <br />
                  есть начало.
                </h1>
                <p>Выберите документ слева или создайте новую сцену.</p>
                <button
                  className="primary"
                  disabled={busy}
                  onClick={() => setDialog("scene")}
                >
                  Создать сцену
                </button>
              </div>
            )}
          </main>
          <aside className="inspector" id="version-history">
            <div
              className="inspector-tabs"
              role="tablist"
              aria-label="Инспектор"
            >
              <button
                role="tab"
                aria-selected={inspectorTab === "ai"}
                onClick={() => setInspectorTab("ai")}
              >
                ИИ
              </button>
              <button
                role="tab"
                aria-selected={inspectorTab === "history"}
                onClick={() => setInspectorTab("history")}
              >
                История
              </button>
            </div>
            {inspectorTab === "ai" ? (
              <AiPanel
                key={`${current?.document.id ?? "none"}:${editorEpoch}`}
                document={
                  current
                    ? { ...current.document, content: current.content }
                    : null
                }
                available={available}
                port={aiPort}
                contextOptions={documents}
                loadContext={loadAiContext}
                onApply={applyAiProposal}
              />
            ) : (
              <section className="history-panel">
                <p className="eyebrow">РАБОЧАЯ ОБЛАСТЬ</p>
                <h2>История версий</h2>
                <p className="muted">Каждое сохранение — точка возврата.</p>
                {current ? (
                  <>
                    <button
                      className="wide"
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          await flush();
                          setVersions(await port.versions(current.document.id));
                        })
                      }
                    >
                      Обновить историю
                    </button>
                    <div className="version-list">
                      {versions.map((version) => (
                        <button
                          key={version.revision}
                          disabled={
                            busy ||
                            version.revision === current.document.revision
                          }
                          onClick={() =>
                            void run(async () => {
                              await flush();
                              setPreview({
                                revision: version.revision,
                                content: await port.versionContent(
                                  current.document.id,
                                  version.revision,
                                ),
                              });
                            })
                          }
                        >
                          <span>
                            Версия {version.revision}
                            {version.revision === current.document.revision
                              ? " · текущая"
                              : ""}
                          </span>
                          <time dateTime={version.created_at}>
                            {new Date(version.created_at).toLocaleString(
                              "ru-RU",
                            )}
                          </time>
                          <small>Вы · локально</small>
                        </button>
                      ))}
                    </div>
                    {versions.length > 0 && versions.length % 200 === 0 && (
                      <button
                        disabled={busy}
                        onClick={() =>
                          void run(async () => {
                            const more = await port.versions(
                              current.document.id,
                              versions.length,
                            );
                            setVersions([...versions, ...more]);
                          })
                        }
                      >
                        Более ранние версии
                      </button>
                    )}
                  </>
                ) : (
                  <p className="empty-small">
                    История появится после выбора документа.
                  </p>
                )}
              </section>
            )}
          </aside>
        </div>
      )}
      {recoveryOpen && (
        <RecoveryDialog
          port={port}
          project={project}
          beforeAction={flush}
          onClose={() => setRecoveryOpen(false)}
          onBusyChange={(value) => {
            recoveryBusy.current = value;
          }}
          onProjectRestored={activateProject}
          onCheckpointRestored={async () => {
            if (session.current)
              await select(await port.read(session.current.document.id));
            setFolders([]);
            setQuery("");
            setSearching(false);
            await refresh(null);
          }}
        />
      )}
      {dialog && (
        <NameDialog
          title={
            dialog === "project"
              ? "Новый проект"
              : dialog === "folder"
                ? "Новая папка"
                : dialog === "note"
                  ? "Новая заметка"
                  : "Новая сцена"
          }
          onCancel={() => setDialog(null)}
          onSubmit={(name) => {
            const kind = dialog;
            setDialog(null);
            void run(async () => {
              if (kind === "project") await chooseProject(name);
              else {
                await flush();
                const doc = await port.createDocument(name, kind, parent);
                await refresh();
                setQuery("");
                setSearching(false);
                if (kind !== "folder") await select(doc);
              }
            });
          }}
        />
      )}
      {renameOpen && current && (
        <NameDialog
          title="Переименовать документ"
          initialValue={current.document.title}
          submitLabel="Переименовать"
          onCancel={() => setRenameOpen(false)}
          onSubmit={(title) => {
            setRenameOpen(false);
            void run(async () => {
              await flush();
              const source = session.current?.document;
              if (!source) return;
              const renamed = await port.renameDocument(
                source.id,
                title,
                source.revision,
                crypto.randomUUID(),
              );
              await refresh(source.parent_id);
              await select(renamed);
            });
          }}
        />
      )}
      {move && (
        <MoveDialog
          document={move.document}
          destinations={move.destinations}
          onCancel={() => setMove(null)}
          onSubmit={(parentId) => {
            const source = move.document;
            setMove(null);
            void run(async () => {
              const moved = await port.moveDocument({
                command_id: crypto.randomUUID(),
                document_id: source.id,
                parent_id: parentId,
                expected_revision: source.revision,
              });
              setFolders([]);
              setQuery("");
              setSearching(false);
              await refresh(null);
              if (session.current?.document.id === moved.id)
                await select(moved);
            });
          }}
        />
      )}
      {preview && current && (
        <RestoreDialog
          revision={preview.revision}
          oldText={preview.content}
          currentText={current.content}
          onCancel={() => setPreview(null)}
          onRestore={() => {
            const revision = preview.revision;
            setPreview(null);
            void run(async () => {
              await flush();
              const restored = await port.restore(
                current.document.id,
                revision,
                current.document.revision,
                crypto.randomUUID(),
              );
              session.current?.dispose();
              session.current = null;
              render();
              await select(restored);
            });
          }}
        />
      )}
    </div>
  );
}
