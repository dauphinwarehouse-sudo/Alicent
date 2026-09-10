import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  ProviderGenerationEvent,
  ProviderGenerationPort,
  ProviderGenerationResult,
} from "@alicent/contracts";
import { ensureProviderCapability } from "./provider-settings-bridge";

/** Native-only boundary. Provider credentials never enter the renderer. */
export const providerGenerationBridge: ProviderGenerationPort = {
  async generate({ requestId, ...request }, onEvent) {
    await ensureProviderCapability();
    const channel = new Channel<ProviderGenerationEvent>();
    channel.onmessage = onEvent;
    return invoke<ProviderGenerationResult>("generate_provider_text", {
      requestId,
      request,
      onEvent: channel,
    });
  },
  cancel: (requestId) =>
    invoke<void>("cancel_provider_generation", { requestId }),
};
