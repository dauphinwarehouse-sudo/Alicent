import { useCallback, useEffect, useId, useRef, useState } from "react";
import type {
  ProviderKind,
  ProviderSettingsDraft,
  ProviderSettingsPort,
  ProviderSettingsSnapshot,
} from "../../../packages/contracts/src/provider-settings";
import { providerSettingsBridge } from "./provider-settings-bridge";
import "./ProviderSettings.css";

const defaults: Record<
  ProviderKind,
  Pick<ProviderSettingsDraft, "endpoint" | "model">
> = {
  openai: { endpoint: "https://api.openai.com/v1", model: "gpt-4.1" },
  anthropic: {
    endpoint: "https://api.anthropic.com",
    model: "claude-sonnet-4-20250514",
  },
};
const initial: ProviderSettingsSnapshot = {
  provider: "openai",
  ...defaults.openai,
  privacy: "strict",
  credentialStored: false,
};
type Operation = "loading" | "idle" | "saving" | "testing" | "deleting";
type Notice = { kind: "neutral" | "success" | "error"; text: string };

function SettingsDialog({
  port,
  close,
}: {
  port: ProviderSettingsPort;
  close: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const secretLabelId = useId();
  const secretStateId = useId();
  // The last snapshot the backend confirmed. Credential state is only known
  // for that provider, so it must not be inferred for any other one.
  const [saved, setSaved] = useState<ProviderSettingsSnapshot | null>(null);
  const [settings, setSettings] = useState(initial);
  const [secret, setSecret] = useState("");
  const [operation, setOperation] = useState<Operation>("loading");
  const [loadFailed, setLoadFailed] = useState(false);
  const [notice, setNotice] = useState<Notice>({
    kind: "neutral",
    text: "Настройки ещё не проверены.",
  });
  useEffect(() => {
    ref.current?.showModal();
    let active = true;
    void port
      .loadSettings()
      .then((value) => {
        if (!active) return;
        setSaved(value);
        setSettings(value);
      })
      .catch(() => {
        if (!active) return;
        // Showing defaults as if they were the current configuration would
        // let one accidental save overwrite the real settings.
        setLoadFailed(true);
        setNotice({
          kind: "error",
          text: "Не удалось загрузить настройки. Сохранение отключено, чтобы не перезаписать текущую конфигурацию.",
        });
      })
      .finally(() => {
        if (active) setOperation("idle");
      });
    return () => {
      active = false;
    };
  }, [port]);
  const busy = operation !== "idle" && operation !== "loading";
  const credentialKnown =
    saved !== null && saved.provider === settings.provider;
  const credentialStored =
    saved !== null && saved.provider === settings.provider
      ? saved.credentialStored
      : false;
  const draft: ProviderSettingsDraft = {
    provider: settings.provider,
    endpoint: settings.endpoint,
    model: settings.model,
    privacy: settings.privacy,
  };
  function chooseProvider(provider: ProviderKind) {
    setSettings((value) => {
      const previous = defaults[value.provider];
      const next = defaults[provider];
      return {
        ...value,
        provider,
        // Keep values the user typed; only replace untouched defaults.
        endpoint:
          value.endpoint === previous.endpoint ? next.endpoint : value.endpoint,
        model: value.model === previous.model ? next.model : value.model,
        credentialStored:
          saved !== null && saved.provider === provider
            ? saved.credentialStored
            : false,
      };
    });
    setSecret("");
    setNotice(
      saved !== null && saved.provider === provider
        ? {
            kind: "neutral",
            text: "Восстановлены сохранённые настройки провайдера.",
          }
        : {
            kind: "neutral",
            text: "Ключ для этого провайдера неизвестен. Сохраните изменения перед проверкой.",
          },
    );
  }
  async function save() {
    if (loadFailed) return;
    const pending = secret.trim();
    setOperation("saving");
    setNotice({ kind: "neutral", text: "Сохранение…" });
    try {
      const snapshot = await port.saveSettings(draft);
      setSaved(snapshot);
      setSettings(snapshot);
      if (!pending) {
        setNotice({ kind: "success", text: "Настройки сохранены." });
        return;
      }
      // Storing the key is a second, independent side effect: its failure
      // must not be reported as a fully successful save.
      try {
        await port.storeCredential(draft.provider, pending);
        const stored = { ...snapshot, credentialStored: true };
        setSaved(stored);
        setSettings(stored);
        setNotice({ kind: "success", text: "Настройки и ключ сохранены." });
      } catch {
        setNotice({
          kind: "error",
          text: "Настройки сохранены, но ключ записать не удалось. Введите ключ ещё раз.",
        });
      }
    } catch {
      setNotice({
        kind: "error",
        text: "Не удалось сохранить настройки. Секрет не показан и не записан в журнал.",
      });
    } finally {
      setSecret("");
      setOperation("idle");
    }
  }
  async function removeCredential() {
    setOperation("deleting");
    setNotice({ kind: "neutral", text: "Удаление ключа…" });
    try {
      await port.deleteCredential(settings.provider);
      setSettings((value) => ({ ...value, credentialStored: false }));
      setSaved((value) =>
        value === null || value.provider !== settings.provider
          ? value
          : { ...value, credentialStored: false },
      );
      setNotice({ kind: "success", text: "Сохранённый ключ удалён." });
    } catch {
      setNotice({
        kind: "error",
        text: "Не удалось удалить сохранённый ключ.",
      });
    } finally {
      setSecret("");
      setOperation("idle");
    }
  }
  async function testConnection() {
    setOperation("testing");
    setNotice({ kind: "neutral", text: "Проверяем соединение…" });
    try {
      const result = await port.testConnection(draft);
      setNotice({
        kind: result.ok ? "success" : "error",
        text: result.ok
          ? result.latencyMs
            ? `Соединение установлено (${result.latencyMs} мс).`
            : "Соединение установлено."
          : "Провайдер отклонил проверку соединения.",
      });
    } catch {
      setNotice({
        kind: "error",
        text: "Проверка соединения не выполнена. Проверьте endpoint, модель и ключ.",
      });
    } finally {
      setOperation("idle");
    }
  }
  return (
    <dialog
      ref={ref}
      className="provider-settings-dialog"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      aria-busy={operation === "loading" || busy}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) close();
      }}
      onClick={(event) => {
        // A click that lands on the dialog element itself is a backdrop click.
        if (!busy && event.target === ref.current) close();
      }}
    >
      {operation === "loading" ? (
        <p className="provider-settings-loading" role="status">
          Загружаем настройки провайдера…
        </p>
      ) : (
        <form
          className="provider-settings-form"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <header className="provider-settings-header">
            <h2 id={titleId}>ИИ-провайдер</h2>
            <p id={descriptionId}>
              Ключ хранится только в системном хранилище учётных данных и
              никогда не отображается.
            </p>
          </header>
          <fieldset
            className="provider-settings-fields"
            disabled={busy || loadFailed}
          >
            <legend>Подключение</legend>
            <label className="provider-settings-field">
              Провайдер
              <select
                value={settings.provider}
                onChange={(event) =>
                  chooseProvider(event.target.value as ProviderKind)
                }
              >
                <option value="openai">OpenAI</option>
                <option value="anthropic">Anthropic</option>
              </select>
            </label>
            <label className="provider-settings-field">
              Модель
              <input
                required
                value={settings.model}
                autoComplete="off"
                onChange={(event) =>
                  setSettings({ ...settings, model: event.target.value })
                }
              />
            </label>
            <label className="provider-settings-field provider-settings-field-wide">
              Endpoint
              <input
                required
                type="url"
                value={settings.endpoint}
                autoComplete="url"
                onChange={(event) =>
                  setSettings({ ...settings, endpoint: event.target.value })
                }
              />
            </label>
            <label className="provider-settings-field">
              Приватность
              <select
                value={settings.privacy}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    privacy: event.target
                      .value as ProviderSettingsDraft["privacy"],
                  })
                }
              >
                <option value="strict">Строгая — минимум данных</option>
                <option value="balanced">Обычная</option>
              </select>
            </label>
            <label className="provider-settings-field">
              <span id={secretLabelId}>API-ключ</span>
              <input
                type="password"
                value={secret}
                autoComplete="new-password"
                spellCheck={false}
                aria-labelledby={secretLabelId}
                aria-describedby={secretStateId}
                onChange={(event) => setSecret(event.target.value)}
                placeholder={
                  credentialStored
                    ? "Сохранён в системном хранилище"
                    : "Ключ не сохранён"
                }
              />
              <span
                id={secretStateId}
                className="provider-settings-secret-state"
              >
                {credentialStored
                  ? "Ключ сохранён. Введите новый только для замены."
                  : credentialKnown
                    ? "Ключ отсутствует."
                    : "Состояние ключа для этого провайдера неизвестно до сохранения."}
              </span>
            </label>
          </fieldset>
          <p className="provider-settings-help">
            Проверка использует сохранённый ключ. Новый ключ сначала сохраните.
          </p>
          {notice.kind === "error" ? (
            <p
              className="provider-settings-status"
              data-kind="error"
              role="alert"
            >
              {notice.text}
            </p>
          ) : (
            <p
              className="provider-settings-status"
              data-kind={notice.kind}
              role="status"
              aria-live="polite"
            >
              {notice.text}
            </p>
          )}
          <div className="provider-settings-actions">
            <button
              className="provider-settings-danger"
              type="button"
              disabled={busy || !credentialStored}
              onClick={() => void removeCredential()}
            >
              Удалить ключ
            </button>
            <button type="button" disabled={busy} onClick={close}>
              Закрыть
            </button>
            <button
              type="button"
              disabled={busy || !credentialStored || secret.length > 0}
              onClick={() => void testConnection()}
            >
              {operation === "testing" ? "Проверяем…" : "Проверить соединение"}
            </button>
            <button
              className="primary"
              type="submit"
              disabled={busy || loadFailed}
            >
              {operation === "saving" ? "Сохраняем…" : "Сохранить"}
            </button>
          </div>
        </form>
      )}
    </dialog>
  );
}

export function ProviderSettingsLauncher({
  port = providerSettingsBridge,
  available = true,
}: {
  port?: ProviderSettingsPort;
  available?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => {
    setOpen(false);
    // Keyboard users must not be dropped at the top of the document.
    trigger.current?.focus();
  }, []);
  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="provider-settings-trigger"
        disabled={!available}
        title={available ? undefined : "Доступно только в приложении"}
        onClick={() => setOpen(true)}
      >
        ИИ-провайдер
      </button>
      {open && <SettingsDialog port={port} close={close} />}
    </>
  );
}
