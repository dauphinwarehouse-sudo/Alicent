import { useEffect, useRef, useState } from "react";
import type {
  Checkpoint,
  CheckpointPreview,
  Project,
  ProjectPort,
} from "@alicent/contracts";

/** Native-only operations; the caller freezes editing while this modal is open. */
export function RecoveryDialog({
  port,
  project,
  beforeAction,
  onClose,
  onProjectRestored,
  onCheckpointRestored,
  onBusyChange,
}: {
  port: ProjectPort;
  project: Project | null;
  beforeAction: () => Promise<void>;
  onClose: () => void;
  onProjectRestored: (project: Project) => Promise<void>;
  onCheckpointRestored: () => Promise<void>;
  onBusyChange: (busy: boolean) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const running = useRef(false);
  const [busy, setBusy] = useState(false);
  const [cancellable, setCancellable] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [items, setItems] = useState<Checkpoint[]>([]);
  const [more, setMore] = useState(false);
  const [name, setName] = useState("");
  const [preview, setPreview] = useState<CheckpointPreview | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  function fail(e: unknown) {
    setError(
      typeof e === "string"
        ? e
        : e instanceof Error
          ? e.message
          : "Операция не выполнена.",
    );
  }
  async function load(offset = 0) {
    const rows = await port.checkpoints(offset);
    setItems((old) => (offset ? [...old, ...rows] : rows));
    setMore(rows.length === 200);
  }
  useEffect(() => {
    dialog.current?.showModal();
    let active = true;
    if (project)
      void port
        .checkpoints()
        .then((rows) => {
          if (active) {
            setItems(rows);
            setMore(rows.length === 200);
          }
        })
        .catch((e) => {
          if (active) fail(e);
        });
    return () => {
      active = false;
    };
  }, [port, project]);
  async function run(action: () => Promise<void>, canCancel = false) {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    onBusyChange(true);
    setError("");
    setMessage("");
    setCancelRequested(false);
    try {
      await beforeAction();
      setCancellable(canCancel);
      await action();
    } catch (e) {
      fail(e);
    } finally {
      running.current = false;
      setBusy(false);
      setCancellable(false);
      onBusyChange(false);
    }
  }
  async function showPreview(id: string, offset = 0) {
    const next = await port.checkpointPreview(id, offset);
    setConfirmed(false);
    setPreview((old) =>
      offset &&
      old?.checkpoint.id === id &&
      old.project_revision === next.project_revision
        ? { ...next, documents: [...old.documents, ...next.documents] }
        : next,
    );
  }
  return (
    <dialog
      ref={dialog}
      className="recovery-dialog"
      aria-labelledby="recovery-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!running.current) onClose();
      }}
    >
      <div className="recovery-heading">
        <div>
          <p className="eyebrow">ALICENT / СОХРАННОСТЬ</p>
          <h2 id="recovery-title">Ваша рукопись под защитой</h2>
        </div>
        <button
          onClick={onClose}
          disabled={busy}
          aria-label="Закрыть сохранность"
        >
          Закрыть
        </button>
      </div>
      <p className="recovery-intro">
        Копия защищает весь проект. Контрольная точка позволяет вернуть тексты к
        выбранному этапу.
      </p>
      {error && (
        <div role="alert" className="recovery-error">
          {error}
        </div>
      )}
      {message && (
        <div role="status" className="recovery-message">
          {message}
        </div>
      )}
      {busy && (
        <div role="status" className="recovery-message">
          {cancelRequested
            ? "Отмена запрошена. Дождитесь результата операции."
            : "Выполняется операция… Не закрывайте приложение."}
          {cancellable && (
            <button
              disabled={cancelRequested}
              onClick={() => {
                setCancelRequested(true);
                void port.cancelRecovery().catch(fail);
              }}
            >
              Отменить операцию
            </button>
          )}
        </div>
      )}
      <section className="recovery-section" aria-labelledby="backup-title">
        <h3 id="backup-title">Резервная копия проекта</h3>
        <p>
          Тексты, история и контрольные точки — в одном файле. Выберите
          отдельный диск для защиты от поломки компьютера. Файл не зашифрован.
        </p>
        <div className="actions">
          <button
            disabled={!project || busy}
            onClick={() =>
              void run(async () => {
                const backup = await port.backupProject();
                setMessage(
                  backup
                    ? `Копия проверена и сохранена: ${backup.path}`
                    : "Создание копии отменено.",
                );
              }, true)
            }
          >
            Создать резервную копию
          </button>
          <button
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const restored = await port.restoreBackup();
                if (restored) {
                  await onProjectRestored(restored);
                  onClose();
                } else setMessage("Выбор резервной копии отменён.");
              }, true)
            }
          >
            Открыть из резервной копии
          </button>
        </div>
        <p className="recovery-hint">
          Восстановление всегда создаёт новую папку проекта. Исходный проект не
          перезаписывается.
        </p>
      </section>
      {project && (
        <section
          className="recovery-section"
          aria-labelledby="checkpoint-title"
        >
          <h3 id="checkpoint-title">Контрольные точки текста</h3>
          <p>
            Сохраняют версии всех сцен и заметок, но не расположение папок и не
            названия. Новые документы при возврате останутся на месте.
          </p>
          <form
            className="checkpoint-form"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await port.createCheckpoint(crypto.randomUUID(), name.trim());
                setName("");
                setPreview(null);
                await load();
                setMessage("Контрольная точка создана.");
              });
            }}
          >
            <label>
              Название точки
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                maxLength={200}
                required
                disabled={busy}
                placeholder="Например, До редактуры"
              />
            </label>
            <button className="primary" disabled={busy || !name.trim()}>
              Создать точку
            </button>
          </form>
          <div className="checkpoint-list">
            {items.map((cp) => (
              <button
                key={cp.id}
                disabled={busy}
                aria-pressed={preview?.checkpoint.id === cp.id}
                onClick={() => void run(() => showPreview(cp.id))}
              >
                <strong>{cp.name}</strong>
                <span>
                  {new Date(cp.created_at).toLocaleString("ru-RU")} ·
                  Документов: {cp.document_count}
                </span>
              </button>
            ))}
          </div>
          {!items.length && (
            <p className="recovery-hint">
              Точек пока нет. Создайте первую перед большой правкой.
            </p>
          )}
          {more && (
            <button
              disabled={busy}
              onClick={() => void run(() => load(items.length))}
            >
              Более ранние точки
            </button>
          )}
          {preview && (
            <div
              className="checkpoint-preview"
              aria-labelledby="checkpoint-preview-title"
            >
              <h3 id="checkpoint-preview-title">
                Возврат к «{preview.checkpoint.name}»
              </h3>
              <p>
                Изменится документов: <strong>{preview.changed_count}</strong>.
                Более новых документов останется:{" "}
                <strong>{preview.newer_document_count}</strong>.
              </p>
              <ul>
                {preview.documents.map((doc) => (
                  <li key={doc.id}>
                    <span>{doc.title}</span>
                    <span>
                      {doc.changed
                        ? `Версия ${doc.current_revision} → текст версии ${doc.target_revision}`
                        : "Без изменений"}
                    </span>
                  </li>
                ))}
              </ul>
              {preview.has_more && (
                <button
                  disabled={busy}
                  onClick={() =>
                    void run(() =>
                      showPreview(
                        preview.checkpoint.id,
                        preview.documents.length,
                      ),
                    )
                  }
                >
                  Показать ещё документы
                </button>
              )}
              <p>
                Возврат создаст новые версии. Перед ним автоматически появится
                точка для отмены восстановления. Если проект изменился,
                потребуется заново открыть предпросмотр.
              </p>
              <label className="recovery-confirm">
                <input
                  type="checkbox"
                  checked={confirmed}
                  disabled={busy}
                  onChange={(e) => setConfirmed(e.target.checked)}
                />
                Подтверждаю восстановление текстов всей рукописи
              </label>
              <button
                className="primary"
                disabled={busy || !confirmed || !preview.changed_count}
                onClick={() =>
                  void run(async () => {
                    const result = await port.restoreCheckpoint(
                      preview.checkpoint.id,
                      preview.project_revision,
                      crypto.randomUUID(),
                    );
                    setPreview(null);
                    setConfirmed(false);
                    await onCheckpointRestored();
                    await load();
                    setMessage(
                      `Восстановлено документов: ${result.changed_count}. ${result.undo_checkpoint_id ? "Точка «Перед восстановлением» позволяет отменить этот возврат." : "Тексты не изменились."}`,
                    );
                  }, true)
                }
              >
                Восстановить тексты
              </button>
            </div>
          )}
        </section>
      )}
    </dialog>
  );
}
