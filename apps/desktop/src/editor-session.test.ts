import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Document, SaveDocument } from '@alicent/contracts';
import { EditorSession } from './editor-session';
const doc: Document = { id:'one',parent_id:null,title:'Глава',kind:'scene',content:'Начало',revision:0,updated_at:'2026-01-01T00:00:00Z' };
const saved = (cmd: SaveDocument): Document => ({ ...doc, content:cmd.content, revision:cmd.expected_revision+1 });
afterEach(() => vi.useRealTimers());
describe('serialized autosave', () => {
  it('debounces edits and commits only the latest text', async () => {
    vi.useFakeTimers(); const save = vi.fn(async (cmd: SaveDocument) => saved(cmd));
    const session = new EditorSession(doc,{save},vi.fn());
    session.edit('Один'); session.edit('Два');
    await vi.advanceTimersByTimeAsync(650);
    expect(save).toHaveBeenCalledTimes(1); expect(save.mock.calls[0][0].content).toBe('Два');
    expect(session.state).toBe('saved'); expect(session.dirty).toBe(false); session.dispose();
  });
  it('serializes edits arriving during an in-flight save', async () => {
    let resolve!: (value: Document) => void;
    const save = vi.fn<(cmd: SaveDocument) => Promise<Document>>()
      .mockImplementationOnce(() => new Promise(r => { resolve = r; }))
      .mockImplementation(async cmd => saved(cmd));
    const session = new EditorSession(doc,{save},vi.fn());
    session.edit('Первая'); const first = session.flush(); session.edit('Вторая');
    expect(session.flush()).toBe(first); expect(save).toHaveBeenCalledTimes(1);
    resolve(saved(save.mock.calls[0][0])); await first;
    expect(save).toHaveBeenCalledTimes(2); expect(save.mock.calls[1][0].expected_revision).toBe(1);
    expect(session.document.content).toBe('Вторая'); expect(session.document.revision).toBe(2); session.dispose();
  });
  it('retains a failed draft, rejects navigation flush, supports retry', async () => {
    const save = vi.fn<(cmd: SaveDocument) => Promise<Document>>()
      .mockRejectedValueOnce('Диск заполнен').mockImplementation(async cmd => saved(cmd));
    const session = new EditorSession(doc,{save},vi.fn()); session.edit('Не потерять');
    await expect(session.flush()).rejects.toBe('Диск заполнен');
    expect(session.content).toBe('Не потерять'); expect(session.document.content).toBe('Начало');
    expect(session.state).toBe('error'); expect(session.dirty).toBe(true);
    await session.flush(); expect(session.document.content).toBe('Не потерять'); session.dispose();
  });
  it('does not produce a version for unchanged text', async () => {
    const save = vi.fn(); const session = new EditorSession(doc,{save},vi.fn());
    await session.flush(); expect(save).not.toHaveBeenCalled(); session.dispose();
  });
  it('manual save cancels the debounce timer', async () => {
    vi.useFakeTimers(); const save = vi.fn(async (cmd: SaveDocument) => saved(cmd));
    const session = new EditorSession(doc,{save},vi.fn()); session.edit('Сохранить');
    await session.flush(); await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledTimes(1); session.dispose();
  });
});
it('reuses command identity after a lost acknowledgement before saving newer edits', async () => {
  const save = vi.fn<(cmd: SaveDocument) => Promise<Document>>()
    .mockRejectedValueOnce('Ответ потерян').mockImplementation(async cmd => saved(cmd));
  const session = new EditorSession(doc,{save},vi.fn()); session.edit('Первая');
  await expect(session.flush()).rejects.toBe('Ответ потерян'); session.edit('Вторая');
  await session.flush();
  expect(save.mock.calls[0][0]).toEqual(save.mock.calls[1][0]);
  expect(save.mock.calls[2][0].expected_revision).toBe(1);
  expect(session.document.content).toBe('Вторая'); session.dispose();
});
