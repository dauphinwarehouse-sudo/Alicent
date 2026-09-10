import { invoke } from "@tauri-apps/api/core";
import type {
  ProviderConnectionResult,
  ProviderKind,
  ProviderSettingsDraft,
  ProviderSettingsPort,
  ProviderSettingsSnapshot,
} from "../../../packages/contracts/src/provider-settings";

type ProviderCapabilities = {
  protocolVersion: number;
  credentialVaultAvailable: boolean;
  providers: ProviderKind[];
  boundedStreaming: boolean;
  connectionTest: string;
  loopbackHttp: string;
};

let handshake: Promise<void> | undefined;

export function ensureProviderCapability(): Promise<void> {
  if (!handshake) {
    handshake = invoke<ProviderCapabilities>("provider_capabilities")
      .then((capabilities) => {
        if (
          capabilities.protocolVersion !== 1 ||
          !capabilities.credentialVaultAvailable ||
          !capabilities.boundedStreaming ||
          capabilities.connectionTest !== "models-metadata" ||
          capabilities.loopbackHttp !== "disabled-in-production" ||
          !capabilities.providers.includes("openai") ||
          !capabilities.providers.includes("anthropic")
        ) {
          throw new Error("provider-capability-unavailable");
        }
      })
      .catch(() => {
        handshake = undefined;
        throw new Error("provider-capability-unavailable");
      });
  }
  return handshake;
}

function afterHandshake<T>(operation: () => Promise<T>): Promise<T> {
  return ensureProviderCapability().then(operation);
}

/** Native-only boundary. Stored credentials are never read into the renderer. */
export const providerSettingsBridge: ProviderSettingsPort = {
  loadSettings: () =>
    afterHandshake(() =>
      invoke<ProviderSettingsSnapshot>("load_provider_settings"),
    ),
  saveSettings: (settings) =>
    afterHandshake(() =>
      invoke<ProviderSettingsSnapshot>("save_provider_settings", { settings }),
    ),
  storeCredential: (provider: ProviderKind, secret: string) =>
    afterHandshake(() =>
      invoke<void>("store_provider_credential", { provider, secret }),
    ),
  deleteCredential: (provider: ProviderKind) =>
    afterHandshake(() =>
      invoke<void>("delete_provider_credential", { provider }),
    ),
  testConnection: (settings: ProviderSettingsDraft) =>
    afterHandshake(() =>
      invoke<ProviderConnectionResult>("test_provider_connection", {
        settings,
      }),
    ),
};
