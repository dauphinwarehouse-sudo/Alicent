import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Document, ProjectPort } from '@alicent/contracts';
import { App } from './App';
// Component test double only. Real CodeMirror is exercised in browser smoke tests.
vi.mock('./Editor', () => ({ Editor: ({ initial, onChange }: {initial:string;onChange:(s:string)=>void}) => <textarea aria-label="Текст документа" defaultValue={initial} onChange={e=>onChange(e.target.value)}/> }));
afterEach(cleanup);
const doc: Document = { id:'a',title:'Первая глава',kind:'scene',parent_id:null,content:'Исходный текст',revision:0,updated_at:'2026-01-01T00:00:00Z' };
function port(): ProjectPort {
  return {
    createProject:vi.fn(), openProject:vi.fn(async()=>({id:'p',title:'Тестовая рукопись',schema_version:1,created_at:'2026-01-01T00:00:00Z'})),
    list:vi.fn(async()=>[doc,{...doc,id:'b',title:'Вторая глава'}]), createDocument:vi.fn(),
    read:vi.fn(async id=>({...doc,id})), save:vi.fn(async cmd=>({...doc,content:cmd.content,revision:1})),
    search:vi.fn(async()=>[]), versions:vi.fn(async()=>[]), versionContent:vi.fn(), restore:vi.fn(),
  };
}
it('browser preview is explicit and cannot silently persist projects', () => {
  render(<App available={false}/>);
  expect(screen.getByText('Предпросмотр интерфейса')).toBeTruthy();
  expect((screen.getByRole('button',{name:'Новый проект'}) as HTMLButtonElement).disabled).toBe(true);
});
it('opens a project and document through its port', async () => {
  const user = userEvent.setup(); const api = port(); render(<App port={api} available/>);
  await user.click(screen.getByRole('button',{name:'Открыть проект'}));
  await user.click(await screen.findByRole('button',{name:/Первая глава/}));
  expect((await screen.findByLabelText('Текст документа') as HTMLTextAreaElement).value).toBe('Исходный текст');
});
it('failed save prevents document switching and retains the draft', async () => {
  const user = userEvent.setup(); const api = port(); api.save = vi.fn().mockRejectedValue('Конфликт версий');
  render(<App port={api} available/>);
  await user.click(screen.getByRole('button',{name:'Открыть проект'}));
  await user.click(await screen.findByRole('button',{name:/Первая глава/}));
  await user.type(await screen.findByLabelText('Текст документа'),' — новая строка');
  await user.click(screen.getByRole('button',{name:/Вторая глава/}));
  await waitFor(()=>expect(api.save).toHaveBeenCalled());
  expect(api.read).toHaveBeenCalledTimes(1);
  expect((screen.getByLabelText('Текст документа') as HTMLTextAreaElement).value).toContain('новая строка');
});
