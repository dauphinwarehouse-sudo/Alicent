import { useEffect, useReducer, useRef, useState } from "react";
import { diffWords } from "diff";
import type {
  Document,
  DocumentKind,
  DocumentSummary,
  Project,
  ProjectPort,
  VersionSummary,
} from "@alicent/contracts";
import { api, desktopAvailable } from "./api";
import { Editor } from "./Editor";
import { EditorSession } from "./editor-session";

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
  onCancel,
  onSubmit,
}: {
  title: string;
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
            placeholder="Например, Северный ветер"
          />
        </label>
        <div className="actions">
          <button type="button" onClick={onCancel}>
            Отмена
          </button>
          <button className="primary" type="submit">
            Создать
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
}: {
  port?: ProjectPort;
  available?: boolean;
}) {
  const [project, setProject] = useState<Project | null>(null);
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [folders, setFolders] = useState<{ id: string; title: string }[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const [dialog, setDialog] = useState<"project" | DocumentKind | null>(null);
  const [versions, setVersions] = useState<VersionSummary[]>([]);
  const [preview, setPreview] = useState<{
    revision: number;
    content: string;
  } | null>(null);
  const [focus, setFocus] = useState(false);
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
  async function refresh(parentId = parent) {
    const rows = await port.list(parentId);
    setDocuments(rows);
    setHasMore(rows.length === 200);
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
    session.current?.dispose();
    session.current = null;
    setProject(next);
    setFolders([]);
    setQuery("");
    setSearching(false);
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
  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (session.current?.dirty) {
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
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            A
          </span>
          <strong>Alicent</strong>
          <span className="build-tag">Локальное ядро · 0.1</span>
        </div>
        <nav aria-label="Проект">
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
            <p className="eyebrow">ВАША ИСТОРИЯ. ВАШЕ ПРОСТРАНСТВО.</p>
            <h1>
              Место, где текст
              <br />
              становится историей.
            </h1>
            <p className="lead">
              Собирайте главы, пишите без отвлечений и возвращайтесь к любой
              сохранённой версии. Произведение остаётся на вашем компьютере.
            </p>
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
            <span className="section-number">01 / ОСНОВА</span>
            <h2>Сначала — текст.</h2>
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
                <h3>История без потерь</h3>
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
              ИИ-провайдеры и агенты — следующие этапы. В этом прототипе сетевых
              запросов к моделям нет.
            </p>
          </section>
        </main>
      ) : (
        <div className="workspace" aria-busy={busy}>
          <aside className="sidebar">
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
              <h3>{searching ? "Результаты поиска" : "Рукопись"}</h3>
              <button
                aria-label="Новая сцена"
                disabled={busy}
                onClick={() => setDialog("scene")}
              >
                +
              </button>
            </div>
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
            <nav className="document-list" aria-label="Документы">
              {documents.map((doc) => (
                <button
                  key={doc.id}
                  title={doc.title}
                  disabled={busy}
                  className={current?.document.id === doc.id ? "selected" : ""}
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
              ))}
              {!documents.length && (
                <p className="empty-small">
                  {searching
                    ? "Совпадений нет. Попробуйте другое слово."
                    : "Здесь пока пусто. Создайте первую сцену."}
                </p>
              )}
              {hasMore && (
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
              {searching && documents.length === 200 && (
                <p className="empty-small">
                  Первые 200 совпадений. Уточните запрос.
                </p>
              )}
            </nav>
            <div className="tree-actions">
              <button disabled={busy} onClick={() => setDialog("folder")}>
                + Папка
              </button>
              <button disabled={busy} onClick={() => setDialog("note")}>
                + Заметка
              </button>
            </div>
            <div className="local-note">
              <span aria-hidden="true">●</span> Локальный проект
              <span>Никакой облачной синхронизации</span>
            </div>
          </aside>
          <main className="writing-pane">
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
                  <button onClick={() => setFocus(!focus)} aria-pressed={focus}>
                    {focus ? "Вернуть панели" : "Фокус"}
                  </button>
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
                  readOnly={busy || !!preview || !!dialog}
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
          <aside className="inspector">
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
                        busy || version.revision === current.document.revision
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
                        {new Date(version.created_at).toLocaleString("ru-RU")}
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
            <div className="next-stage">
              <span className="eyebrow">ДАЛЬШЕ В РАЗРАБОТКЕ</span>
              <h3>Соавтор рядом</h3>
              <p>
                Подключение моделей, агенты и правки с подтверждением. Пока не
                реализовано.
              </p>
            </div>
          </aside>
        </div>
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
