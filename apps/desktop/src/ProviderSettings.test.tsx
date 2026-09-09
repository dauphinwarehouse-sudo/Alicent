import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
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
    testConnection: vi.fn(async () => ({ ok: true, message: "ok", latencyMs: 42 })),
    ...overrides,
  };
}
function supportDialog() {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value() {
      this.setAttribute("open", "");
    },
  });
}
afterEach(() => {
  cleanup();
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
  expect((screen.getByLabelText("API-ключ") as HTMLInputElement).value).toBe("");
});

it("reports connection loading and success", async () => {
  supportDialog();
  const user = userEvent.setup();
  let resolve:
    | ((value: { ok: boolean; message: string; latencyMs: number }) => void)
    | undefined;
  const api = port({
    loadSettings: vi.fn(async () => ({ ...saved, credentialStored: true })),
    testConnection: vi.fn(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    ),
  });
  render(<ProviderSettingsLauncher port={api} />);
  await user.click(screen.getByRole("button", { name: "ИИ-провайдер" }));
  await user.click(
    await screen.findByRole("button", { name: "Проверить соединение" }),
  );
  expect(screen.getByRole("status").textContent).toContain("Проверяем");
  resolve?.({ ok: true, message: "ok", latencyMs: 17 });
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
  await user.click(screen.getByRole("button", { name: "Проверить соединение" }));
  await screen.findByRole("alert");
  expect(document.body.textContent).not.toContain("forbidden-secret");
});
