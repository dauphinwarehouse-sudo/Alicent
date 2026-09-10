import { useEffect, useRef, useState } from "react";
import { diffWords } from "diff";
import type {
  Document,
  DocumentSummary,
  ProviderContextDocument,
  ProviderGenerationPort,
  ProviderGenerationResult,
} from "@alicent/contracts";
import { providerGenerationBridge } from "./provider-generation-bridge";

const MAX_PROMPT = 16_384;
const MAX_DIFF_INPUT = 100_000;
const MAX_CONTEXT_DOCUMENTS = 8;

type Proposal = ProviderGenerationResult & {
  sourceContent: string;
};

const providerErrors: Record<string, string> = {
  CONTEXT_INVALID: "Выберите контекст заново.",
  CONTEXT_STALE: "Один из документов контекста больше недоступен.",
  CONTEXT_FOLDER: "Папку нельзя отправить в контекст модели.",
  CONTEXT_TOO_LARGE:
    "Выбранный контекст превышает лимит 512 КиБ. Уберите часть документов.",
  INVALID_SETTINGS: "Проверьте модель и настройки провайдера.",
  SETTINGS_UNAVAILABLE: "Настройки провайдера сейчас недоступны.",
  PRIVACY_DENIED: "Сетевые запросы запрещены настройками приватности.",
  CUSTOM_ENDPOINT_DENIED: "Пользовательский endpoint запрещён строгим режимом.",
  ENDPOINT_REJECTED: "Endpoint провайдера отклонён.",
  CREDENTIAL_INVALID: "Ключ провайдера имеет неверный формат.",
  CREDENTIAL_MISSING: "Сначала сохраните API-ключ в настройках модели.",
  CREDENTIAL_UNAVAILABLE: "Хранилище API-ключей недоступно.",
  AUTHENTICATION_FAILED: "Провайдер отклонил API-ключ.",
  RATE_LIMITED: "Провайдер ограничил частоту запросов. Попробуйте позже.",
  PROVIDER_REJECTED: "Провайдер отклонил запрос.",
  ENDPOINT_NOT_FOUND: "Endpoint провайдера не найден.",
  PROVIDER_UNAVAILABLE: "Провайдер временно недоступен.",
  UNEXPECTED_RESPONSE: "Провайдер вернул незавершённый или неизвестный ответ.",
  CONNECTION_FAILED: "Не удалось подключиться к провайдеру.",
  TIMEOUT: "Провайдер не ответил вовремя.",
  RESPONSE_TOO_LARGE: "Ответ модели превысил безопасный лимит.",
};

function errorCode(error: unknown): string | undefined {
  if (typeof error === "object" && error && "code" in error) {
    const code = (error as { code?: unknown }).code;
    if (typeof code === "string") return code;
  }
  return undefined;
}

function AiDiffDialog({
  currentText,
  proposal,
  applying,
  onCancel,
  onApply,
}: {
  currentText: string;
  proposal: Proposal;
  applying: boolean;
  onCancel: () => void;
  onApply: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  const parts =
    currentText.length + proposal.text.length < MAX_DIFF_INPUT
      ? diffWords(currentText, proposal.text, {
          intlSegmenter: new Intl.Segmenter("ru", { granularity: "word" }),
          timeout: 200,
          maxEditLength: 10_000,
        })
      : undefined;
  const unchanged = currentText === proposal.text;
  return (
    <dialog
      className="diff-dialog ai-diff-dialog"
      ref={ref}
      onCancel={onCancel}
      aria-labelledby="ai-diff-title"
    >
      <p className="eyebrow">ИИ-РЕДАКТОР / СРАВНЕНИЕ</p>
      <h2 id="ai-diff-title">Применить предложенную редакцию?</h2>
      <p>
        Перед заменой Alicent создаст checkpoint. Текущая версия останется в
        истории.
      </p>
      <p className="diff-key">
        <del>Удаление</del> <ins>Добавление</ins>
      </p>
      <div className="diff-content">
        {parts ? (
          parts.map((part, index) =>
            part.added ? (
              <ins key={index}>{part.value}</ins>
            ) : part.removed ? (
              <del key={index}>{part.value}</del>
            ) : (
              <span key={index}>{part.value}</span>
            ),
          )
        ) : (
          <>
            <p>
              Текст слишком большой для подробного сравнения. Ниже показаны
              первые 10 000 символов предложения.
            </p>
            <pre>{proposal.text.slice(0, 10_000)}</pre>
          </>
        )}
      </div>
      {unchanged && <p>Модель не изменила текст.</p>}
      <div className="actions">
        <button disabled={applying} onClick={onCancel}>
          Оставить текущий текст
        </button>
        <button
          className="primary"
          disabled={applying || unchanged}
          onClick={onApply}
        >
          {applying ? "Применение…" : "Применить как новую версию"}
        </button>
      </div>
    </dialog>
  );
}

export function AiPanel({
  document,
  available,
  port = providerGenerationBridge,
  contextOptions = [],
  pinnedContextIds = [],
  loadContext = async () => [],
  onApply,
}: {
  document: Document | null;
  available: boolean;
  port?: ProviderGenerationPort;
  contextOptions?: DocumentSummary[];
  pinnedContextIds?: string[];
  loadContext?: (ids: string[]) => Promise<ProviderContextDocument[]>;
  onApply: (text: string, sourceContent: string) => Promise<void>;
}) {
  const [prompt, setPrompt] = useState("");
  const [streamed, setStreamed] = useState("");
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [status, setStatus] = useState<
    "idle" | "streaming" | "ready" | "applying"
  >("idle");
  const [error, setError] = useState("");
  const [diffOpen, setDiffOpen] = useState(false);
  const [contextIds, setContextIds] = useState<string[]>([]);
  const requestId = useRef<string | null>(null);
  const contextCandidates = contextOptions.filter(
    (candidate) =>
      candidate.kind !== "folder" &&
      candidate.id !== document?.id &&
      !candidate.ai_context_excluded,
  );
  const contextCandidateKey = contextCandidates
    .map((candidate) => candidate.id)
    .join(",");
  const pinnedContextKey = pinnedContextIds.join(",");

  useEffect(() => {
    const activeRequest = requestId.current;
    if (activeRequest) void port.cancel(activeRequest).catch(() => {});
    requestId.current = null;
    setStreamed("");
    setProposal(null);
    setStatus("idle");
    setError("");
    setDiffOpen(false);
    setContextIds([]);
  }, [document?.id, port]);

  useEffect(() => {
    const availableIds = new Set(
      contextCandidates.map((candidate) => candidate.id),
    );
    setContextIds((ids) =>
      [
        ...new Set([
          ...pinnedContextIds.filter((id) => availableIds.has(id)),
          ...ids.filter((id) => availableIds.has(id)),
        ]),
      ].slice(0, MAX_CONTEXT_DOCUMENTS),
    );
  }, [contextCandidateKey, pinnedContextKey]);

  useEffect(
    () => () => {
      if (requestId.current)
        void port.cancel(requestId.current).catch(() => {});
    },
    [port],
  );

  async function generate() {
    if (
      !document ||
      document.ai_context_excluded ||
      !prompt.trim() ||
      status !== "idle"
    )
      return;
    const id = crypto.randomUUID();
    const sourceContent = document.content;
    requestId.current = id;
    setStreamed("");
    setProposal(null);
    setError("");
    setStatus("streaming");
    try {
      const contextDocuments = await loadContext(contextIds);
      if (requestId.current !== id) return;
      const result = await port.generate(
        {
          requestId: id,
          prompt: prompt.trim(),
          currentDocumentId: document.id,
          documentTitle: document.title,
          documentContent: sourceContent,
          contextDocuments,
          maxOutputTokens: 12_000,
        },
        (event) => {
          if (requestId.current === id && event.type === "delta")
            setStreamed((text) => text + event.text);
        },
      );
      if (requestId.current !== id) return;
      setProposal({ ...result, sourceContent });
      setStreamed(result.text);
      setStatus("ready");
    } catch (cause) {
      if (requestId.current !== id) return;
      const code = errorCode(cause);
      if (code !== "ABORTED")
        setError(
          (code && providerErrors[code]) ??
            "Не удалось получить ответ модели. Текст не изменён.",
        );
      setStatus("idle");
    } finally {
      if (requestId.current === id) requestId.current = null;
    }
  }

  async function cancel() {
    const id = requestId.current;
    if (!id) return;
    requestId.current = null;
    setStatus("idle");
    setError("");
    await port.cancel(id).catch(() => {});
  }

  async function apply() {
    if (!proposal || status !== "ready") return;
    setStatus("applying");
    setError("");
    try {
      await onApply(proposal.text, proposal.sourceContent);
      setDiffOpen(false);
      setProposal(null);
      setStreamed("");
      setPrompt("");
      setStatus("idle");
    } catch {
      setError(
        "Правка не применена. Если текст изменился после запроса, запустите генерацию заново.",
      );
      setStatus("ready");
    }
  }

  const output = proposal?.text ?? streamed;
  return (
    <section className="ai-panel" aria-labelledby="ai-panel-title">
      <p className="eyebrow">ИИ-СОАВТОР</p>
      <h2 id="ai-panel-title">Редактор сцены</h2>
      <p className="muted">
        Опишите правку. Модель предложит полную новую версию — исходник не
        изменится без подтверждения.
      </p>
      {document?.ai_context_excluded && (
        <p className="ai-note">
          Этот документ исключён из ИИ. Разрешите его в панели редактора, чтобы
          запросить правку.
        </p>
      )}
      <label className="ai-prompt">
        Задача
        <textarea
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          maxLength={MAX_PROMPT}
          rows={6}
          disabled={status !== "idle" || document?.ai_context_excluded}
          placeholder="Например: усили конфликт, сохрани стиль и факты сцены"
        />
      </label>
      <fieldset
        className="ai-context"
        disabled={status !== "idle" || document?.ai_context_excluded}
      >
        <legend>Контекст проекта · до {MAX_CONTEXT_DOCUMENTS}</legend>
        {contextCandidates.length ? (
          <div className="ai-context-list">
            {contextCandidates.map((candidate) => {
              const checked = contextIds.includes(candidate.id);
              return (
                <label key={candidate.id}>
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={
                      !checked && contextIds.length >= MAX_CONTEXT_DOCUMENTS
                    }
                    onChange={(event) =>
                      setContextIds((ids) =>
                        event.target.checked
                          ? [...ids, candidate.id].slice(
                              0,
                              MAX_CONTEXT_DOCUMENTS,
                            )
                          : ids.filter((id) => id !== candidate.id),
                      )
                    }
                  />
                  <span title={candidate.title}>
                    {candidate.ai_context_pinned ? "📌 " : ""}
                    {candidate.title}
                  </span>
                </label>
              );
            })}
          </div>
        ) : (
          <p>В текущем списке нет других сцен или заметок.</p>
        )}
        <small>
          Выбрано: {contextIds.length}. Только эти документы будут отправлены
          модели вместе с текущим.
        </small>
      </fieldset>
      <div className="ai-actions">
        {status === "streaming" ? (
          <button onClick={() => void cancel()}>Остановить</button>
        ) : status === "idle" ? (
          <button
            className="primary"
            disabled={
              !available ||
              !document ||
              document.ai_context_excluded ||
              !prompt.trim()
            }
            onClick={() => void generate()}
          >
            Предложить редакцию
          </button>
        ) : null}
      </div>
      {!available && (
        <p className="ai-note">Генерация доступна в Windows-приложении.</p>
      )}
      {!document && (
        <p className="ai-note">Сначала выберите сцену или заметку.</p>
      )}
      {error && (
        <p className="ai-error" role="alert">
          {error}
        </p>
      )}
      {output && (
        <div className="ai-output" aria-live="polite">
          <div className="ai-output-heading">
            <strong>
              {status === "streaming" ? "Модель пишет…" : "Предложение готово"}
            </strong>
            {proposal?.outputTokens != null && (
              <span>
                {proposal.outputTokens.toLocaleString("ru-RU")} токенов
              </span>
            )}
          </div>
          <pre>{output}</pre>
          {proposal && (
            <div className="ai-actions">
              <button onClick={() => setDiffOpen(true)}>
                Сравнить и применить
              </button>
              <button
                onClick={() => {
                  setProposal(null);
                  setStreamed("");
                  setStatus("idle");
                }}
              >
                Отклонить
              </button>
            </div>
          )}
        </div>
      )}
      {diffOpen && proposal && (
        <AiDiffDialog
          currentText={proposal.sourceContent}
          proposal={proposal}
          applying={status === "applying"}
          onCancel={() => setDiffOpen(false)}
          onApply={() => void apply()}
        />
      )}
    </section>
  );
}
