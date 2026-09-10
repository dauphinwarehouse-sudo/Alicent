/** One explicit editor request. Manuscript content is sent only by this call. */
export interface ProviderGenerationRequest {
  requestId: string;
  prompt: string;
  documentTitle: string;
  documentContent: string;
  maxOutputTokens: number;
}

export type ProviderGenerationEvent = {
  type: "delta";
  text: string;
};

export interface ProviderGenerationResult {
  text: string;
  inputTokens: number | null;
  outputTokens: number | null;
}

export interface ProviderGenerationPort {
  generate(
    request: ProviderGenerationRequest,
    onEvent: (event: ProviderGenerationEvent) => void,
  ): Promise<ProviderGenerationResult>;
  cancel(requestId: string): Promise<void>;
}
