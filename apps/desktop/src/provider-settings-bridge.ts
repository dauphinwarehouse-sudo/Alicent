import { invoke } from "@tauri-apps/api/core";
import type {
  ProviderConnectionResult,
  ProviderKind,
  ProviderSettingsDraft,
  ProviderSettingsPort,
  ProviderSettingsSnapshot,
} from "../../../packages/contracts/src/provider-settings";

/**
 * Thin native credential/configuration bridge. It intentionally contains no
 * HTTP adapter and never reads a stored secret back into the renderer.
 */
export const providerSettingsBridge: ProviderSettingsPort = {
  loadSettings: () =>
    invoke<ProviderSettingsSnapshot>("load_provider_settings"),
  saveSettings: (settings) =>
    invoke<ProviderSettingsSnapshot>("save_provider_settings", { settings }),
  storeCredential: (provider: ProviderKind, secret: string) =>
    invoke<void>("store_provider_credential", { provider, secret }),
  deleteCredential: (provider: ProviderKind) =>
    invoke<void>("delete_provider_credential", { provider }),
  testConnection: (settings: ProviderSettingsDraft) =>
    invoke<ProviderConnectionResult>("test_provider_connection", { settings }),
};
