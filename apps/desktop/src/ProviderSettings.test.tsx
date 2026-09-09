import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ProviderConnectionResult,
  ProviderSettingsPort,
  ProviderSettingsSnapshot,
} from "../../../packages/contracts/src/provider-settings";
import { ProviderSettingsLauncher } from "./ProviderSettings";

const saved: ProviderSettingsSnapshot = {
  provider: "openai",
  endpoint: "https://api.openai.com/v1",
  model: "gpt-4.1",
  privacy: "strict",
  credentialStored: false,
};
function port(
  overrides: Partial<ProviderSettingsPort> = {},
): ProviderSettingsPort {
  return {
    loadSettings: vi.fn(async () => saved),
    saveSettings: vi.fn(async (settings) => ({
      ...settings,
      credentialStored: false,
    })),
    storeCredential: vi.fn(async () => undefined),
    deleteCredential: vi.fn(async () => undefined),
    testConnection: vi.fn(async () => ({
      ok: true,
      message: "ok",
      latencyMs: 42,
    })),
    ...overrides,
  };
}
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
  // The patch is global: leaving it in place leaks into unrelated suites.
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

it("never renders a stored secret", async () => {
  supportDialog();
  const user = userEvent.setup();
  render(
    <ProviderSettingsLauncher
      port={port({
        loadSettings: vi.fn(async () => ({ ...saved, credentialStored: true })),
      })}
    />,
  );
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  const secret = (await screen.findByLabelText("API-ключ")) as HTMLInputElement;
  expect(secret.type).toBe("password");
  expect(secret.value).toBe("");
  expect(secret.placeholder).toContain("системном хранилище");
});

it("selects Anthropic and saves the credential through the native port", async () => {
  supportDialog();
  const user = userEvent.setup();
  const api = port();
  render(<ProviderSettingsLauncher port={api} />);
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  await screen.findByLabelText("Провайдер");
  await user.selectOptions(screen.getByLabelText("Провайдер"), "anthropic");
  expect((screen.getByLabelText("Endpoint") as HTMLInputElement).value).toBe(
    "https://api.anthropic.com",
  );
  await user.type(screen.getByLabelText("API-ключ"), "temporary-test-key");
  await user.click(screen.getByRole("button", { name: "Сохранить" }));
  await waitFor(() =>
    expect(api.storeCredential).toHaveBeenCalledWith(
      "anthropic",
      "temporary-test-key",
    ),
  );
  expect((screen.getByLabelText("API-ключ") as HTMLInputElement).value).toBe(
    "",
  );
});

it("keeps the known credential state when the provider is switched back", async () => {
  supportDialog();
  const user = userEvent.setup();
  render(
    <ProviderSettingsLauncher
      port={port({
        loadSettings: vi.fn(async () => ({ ...saved, credentialStored: true })),
      })}
    />,
  );
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  await user.selectOptions(
    await screen.findByLabelText("Провайдер"),
    "anthropic",
  );
  expect(
    (
      screen.getByRole("button", {
        name: "Проверить соединение",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(true);
  await user.selectOptions(screen.getByLabelText("Провайдер"), "openai");
  expect(
    (
      screen.getByRole("button", {
        name: "Проверить соединение",
      }) as HTMLButtonElement
    ).disabled,
  ).toBe(false);
});

it("reports a partially failed save without claiming success", async () => {
  supportDialog();
  const user = userEvent.setup();
  const api = port({
    storeCredential: vi.fn(async () => {
      throw new Error("keychain-locked");
    }),
  });
  render(<ProviderSettingsLauncher port={api} />);
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  await user.type(await screen.findByLabelText("API-ключ"), "temporary-key");
  await user.click(screen.getByRole("button", { name: "Сохранить" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("ключ записать не удалось");
  expect(api.saveSettings).toHaveBeenCalledTimes(1);
  expect(document.body.textContent).not.toContain("keychain-locked");
  expect((screen.getByLabelText("API-ключ") as HTMLInputElement).value).toBe(
    "",
  );
});

it("blocks saving when the settings could not be loaded", async () => {
  supportDialog();
  const user = userEvent.setup();
  const api = port({
    loadSettings: vi.fn(async () => {
      throw new Error("ipc-down");
    }),
  });
  render(<ProviderSettingsLauncher port={api} />);
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("Сохранение отключено");
  expect(
    (screen.getByRole("button", { name: "Сохранить" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(api.saveSettings).not.toHaveBeenCalled();
  expect(document.body.textContent).not.toContain("ipc-down");
});

it("reports connection loading and success", async () => {
  supportDialog();
  const user = userEvent.setup();
  let resolveConnection: (value: ProviderConnectionResult) => void = () =>
    undefined;
  const api = port({
    loadSettings: vi.fn(async () => ({ ...saved, credentialStored: true })),
    testConnection: vi.fn(
      () =>
        new Promise<ProviderConnectionResult>((done) => {
          resolveConnection = done;
        }),
    ),
  });
  render(<ProviderSettingsLauncher port={api} />);
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  await user.click(
    await screen.findByRole("button", { name: "Проверить соединение" }),
  );
  expect(screen.getByRole("status").textContent).toContain("Проверяем");
  resolveConnection({ ok: true, message: "ok", latencyMs: 17 });
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toContain("17 мс"),
  );
});

it("has labelled controls and hides rejected error details", async () => {
  supportDialog();
  const user = userEvent.setup();
  const api = port({
    loadSettings: vi.fn(async () => ({ ...saved, credentialStored: true })),
    testConnection: vi.fn(async () => {
      throw new Error("forbidden-secret");
    }),
  });
  render(<ProviderSettingsLauncher port={api} />);
  await user.tab();
  await user.keyboard("{Enter}");
  const dialog = await screen.findByRole("dialog", { name: "ИИ-провайдер" });
  expect(dialog.getAttribute("aria-describedby")).toBeTruthy();
  expect(screen.getByLabelText("Провайдер")).toBeTruthy();
  expect(screen.getByLabelText("Endpoint")).toBeTruthy();
  expect(screen.getByLabelText("Приватность")).toBeTruthy();
  expect(screen.getByLabelText("API-ключ")).toBeTruthy();
  await user.click(
    screen.getByRole("button", { name: "Проверить соединение" }),
  );
  await screen.findByRole("alert");
  expect(document.body.textContent).not.toContain("forbidden-secret");
});

it("returns focus to the trigger after closing", async () => {
  supportDialog();
  const user = userEvent.setup();
  render(<ProviderSettingsLauncher port={port()} />);
  const trigger = screen.getByRole("button", { name: "ИИ-провайдер" });
  await user.click(trigger);
  await user.click(await screen.findByRole("button", { name: "Закрыть" }));
  await waitFor(() => expect(document.activeElement).toBe(trigger));
});
