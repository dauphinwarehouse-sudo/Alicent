export type ProviderKind = "openai" | "anthropic";

export type ProviderPrivacy = "balanced" | "strict";

/** Persisted provider configuration. Credentials are deliberately excluded. */
export interface ProviderSettingsDraft {
  provider: ProviderKind;
  endpoint: string;
  model: string;
  privacy: ProviderPrivacy;
}

/** Safe-to-render settings snapshot. Never include the stored credential. */
export interface ProviderSettingsSnapshot extends ProviderSettingsDraft {
  credentialStored: boolean;
}

export interface ProviderConnectionResult {
  ok: boolean;
  message: string;
  latencyMs?: number;
}

/** Native boundary used by the desktop UI; HTTP provider adapters live elsewhere. */
export interface ProviderSettingsPort {
  loadSettings(): Promise<ProviderSettingsSnapshot>;
  saveSettings(
    settings: ProviderSettingsDraft,
  ): Promise<ProviderSettingsSnapshot>;
  storeCredential(provider: ProviderKind, secret: string): Promise<void>;
  deleteCredential(provider: ProviderKind): Promise<void>;
  testConnection(
    settings: ProviderSettingsDraft,
  ): Promise<ProviderConnectionResult>;
}
