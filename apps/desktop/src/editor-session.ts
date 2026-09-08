import type { Document, ProjectPort, SaveDocument } from '@alicent/contracts';
export type SaveState = 'saved' | 'dirty' | 'saving' | 'error';
/** One serialized writer per document. Errors retain the draft and block navigation. */
export class EditorSession {
  document: Document;
  content: string;
  state: SaveState = 'saved';
  error = '';
  private timer?: ReturnType<typeof setTimeout>;
  private pending?: Promise<void>;
  private awaitingCommand?: SaveDocument;
  constructor(document: Document, private port: Pick<ProjectPort, 'save'>, private changed: () => void) {
    this.document = document; this.content = document.content;
  }
  get dirty() { return this.content !== this.document.content || !!this.awaitingCommand || this.state === 'saving'; }
  edit(content: string) {
    this.content = content;
    this.state = this.dirty ? 'dirty' : 'saved';
    this.error = '';
    clearTimeout(this.timer);
    if (this.dirty) this.timer = setTimeout(() => { void this.flush().catch(() => { /* State exposes the error. */ }); }, 650);
    this.changed();
  }
  flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.pending) return this.pending;
    this.pending = this.saveLoop().finally(() => { this.pending = undefined; });
    return this.pending;
  }
  private async saveLoop() {
    try {
      while (this.awaitingCommand || this.content !== this.document.content) {
        this.state = 'saving'; this.changed();
        this.awaitingCommand ??= { command_id: crypto.randomUUID(), document_id: this.document.id, expected_revision: this.document.revision, content: this.content };
        this.document = await this.port.save(this.awaitingCommand);
        this.awaitingCommand = undefined;
      }
      this.state = 'saved'; this.error = ''; this.changed();
    } catch (error) {
      this.state = 'error'; this.error = typeof error === 'string' ? error : 'Не удалось сохранить текст. Черновик остаётся в редакторе.';
      this.changed(); throw error;
    }
  }
  dispose() { clearTimeout(this.timer); }
}
