import { useEffect, useId, useRef, useState } from "react";
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
const initialSettings: ProviderSettingsSnapshot = {
  provider: "openai",
  ...defaults.openai,
  privacy: "strict",
  credentialStored: false,
};
type Operation = "idle" | "loading" | "saving" | "testing" | "deleting";
type Notice = { kind: "neutral" | "success" | "error"; text: string };

function ProviderSettingsDialog({ port, onClose }: { port: ProviderSettingsPort; onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const [settings, setSettings] = useState(initialSettings);
  const [secret, setSecret] = useState("");
  const [operation, setOperation] = useState<Operation>("loading");
  const [notice, setNotice] = useState<Notice>({ kind: "neutral", text: "Настройки ещё не проверены." });

  useEffect(() => {
    dialogRef.current?.showModal();
    let active = true;
    void port.loadSettings()
      .then((snapshot) => { if (active) setSettings(snapshot); })
      .catch(() => { if (active) setNotice({ kind: "error", text: "Не удалось загрузить настройки провайдера." }); })
      .finally(() => { if (active) setOperation("idle"); });
    return () => { active = false; };
  }, [port]);

  const busy = operation !== "idle" && operation !== "loading";
  const draft: ProviderSettingsDraft = {
    provider: settings.provider,
    endpoint: settings.endpoint,
    model: settings.model,
    privacy: settings.privacy,
  };
  function chooseProvider(provider: ProviderKind) {
    const previous = defaults[settings.provider];
    setSettings((current) => ({
      ...current,
      provider,
      endpoint: current.endpoint === previous.endpoint ? defaults[provider].endpoint : current.endpoint,
      model: current.model === previous.model ? defaults[provider].model : current.model,
      credentialStored: provider === current.provider ? current.credentialStored : false,
    }));
    setSecret("");
    setNotice({ kind: "neutral", text: "Сохраните изменения перед проверкой." });
  }
  async function save() {
    setOperation("saving");
    setNotice({ kind: "neutral", text: "Сохранение…" });
    try {
      let snapshot = await port.saveSettings(draft);
      const nextSecret = secret.trim();
      if (nextSecret) {
        await port.storeCredential(draft.provider, nextSecret);
        snapshot = { ...snapshot, credentialStored: true };
      }
      setSettings(snapshot);
      setSecret("");
      setNotice({ kind: "success", text: "Настройки сохранены." });
    } catch {
      setNotice({ kind: "error", text: "Не удалось сохранить настройки. Секрет не показан и не записан в журнал." });
    } finally { setOperation("idle"); }
  }
  async function removeCredential() {
    setOperation("deleting");
    setNotice({ kind: "neutral", text: "Удаление ключа…" });
    try {
      await port.deleteCredential(settings.provider);
      setSettings((current) => ({ ...current, credentialStored: false }));
      setSecret("");
      setNotice({ kind: "success", text: "Сохранённый ключ удалён." });
    } catch {
      setNotice({ kind: "error", text: "Не удалось удалить сохранённый ключ." });
    } finally { setOperation("idle"); }
  }
  async function testConnection() {
    setOperation("testing");
    setNotice({ kind: "neutral", text: "Проверяем соединение…" });
    try {
      const result = await port.testConnection(draft);
      setNotice({
        kind: result.ok ? "success" : "error",
        text: result.ok
          ? result.latencyMs ? `Соединение установлено (${result.latencyMs} мс).` : "Соединение установлено."
          : "Провайдер отклонил проверку соединения.",
      });
    } catch {
      setNotice({ kind: "error", text: "Проверка соединения не выполнена. Проверьте endpoint, модель и ключ." });
    } finally { setOperation("idle"); }
  }

  return (
    <dialog ref={dialogRef} className="provider-settings-dialog" aria-labelledby={titleId} aria-describedby={descriptionId} aria-busy={operation === "loading" || busy} onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
      {operation === "loading" ? (
        <p className="provider-settings-loading" role="status">Загружаем настройки провайдера…</p>
      ) : (
        <form className="provider-settings-form" onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <header className="provider-settings-header">
            <h2 id={titleId}>ИИ-провайдер</h2>
            <p id={descriptionId}>Настройка применяется к будущим запросам. Ключ хранится только в системном хранилище учётных данных и никогда не отображается.</p>
          </header>
          <fieldset className="provider-settings-fields" disabled={busy}>
            <legend>Подключение</legend>
            <label className="provider-settings-field">Провайдер
              <select value={settings.provider} onChange={(event) => chooseProvider(event.target.value as ProviderKind)}>
                <option value="openai">OpenAI</option><option value="anthropic">Anthropic</option>
              </select>
            </label>
            <label className="provider-settings-field">Модель
              <input required value={settings.model} onChange={(event) => setSettings({ ...settings, model: event.target.value })} autoComplete="off" />
            </label>
            <label className="provider-settings-field provider-settings-field-wide">Endpoint
              <input required type="url" value={settings.endpoint} onChange={(event) => setSettings({ ...settings, endpoint: event.target.value })} autoComplete="url" />
            </label>
            <label className="provider-settings-field">Приватность
              <select value={settings.privacy} onChange={(event) => setSettings({ ...settings, privacy: event.target.value as ProviderSettingsDraft["privacy"] })}>
                <option value="strict">Строгая — минимум данных</option><option value="balanced">Обычная</option>
              </select>
            </label>
            <label className="provider-settings-field">API-ключ
              <input type="password" value={secret} onChange={(event) => setSecret(event.target.value)} autoComplete="new-password" spellCheck={false} placeholder={settings.credentialStored ? "Сохранён в системном хранилище" : "Ключ не сохранён"} />
              <span className="provider-settings-secret-state">{settings.credentialStored ? "Ключ сохранён. Введите новый только для замены." : "Ключ отсутствует."}</span>
            </label>
          </fieldset>
          <p className="provider-settings-help">Проверка использует сохранённый ключ. Новый введённый ключ сначала нужно сохранить.</p>
          <p className="provider-settings-status" data-kind={notice.kind} role={notice.kind === "error" ? "alert" : "status"} aria-live="polite">{notice.text}</p>
          <div className="provider-settings-actions">
            <button className="provider-settings-danger" type="button" disabled={busy || !settings.credentialStored} onClick={() => void removeCredential()}>Удалить ключ</button>
            <button type="button" disabled={busy} onClick={onClose}>Закрыть</button>
            <button type="button" disabled={busy || !settings.credentialStored || secret.length > 0} onClick={() => void testConnection()}>{operation === "testing" ? "Проверяем…" : "Проверить соединение"}</button>
            <button className="primary" type="submit" disabled={busy}>{operation === "saving" ? "Сохраняем…" : "Сохранить"}</button>
          </div>
        </form>
      )}
    </dialog>
  );
}

export function ProviderSettingsLauncher({ port = providerSettingsBridge, available = true }: { port?: ProviderSettingsPort; available?: boolean }) {
  const [open, setOpen] = useState(false);
  return <>
    <button type="button" className="provider-settings-trigger" disabled={!available} title={available ? undefined : "Доступно только в приложении"} onClick={() => setOpen(true)}>ИИ-провайдер</button>
    {open && <ProviderSettingsDialog port={port} onClose={() => setOpen(false)} />}
  </>;
}
