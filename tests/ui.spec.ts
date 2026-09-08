import { test, expect } from '@playwright/test';
// Browser smoke tests use a test-only port; they are NOT native Windows end-to-end tests.
test('preview accurately disables native project access', async ({page}) => {
  await page.goto('/'); await expect(page.getByText('Предпросмотр интерфейса')).toBeVisible();
  await expect(page.getByRole('button',{name:'Новый проект'})).toBeDisabled();
});
test('edit, autosave, history diff and restore update the actual editor', async ({page}) => {
  await page.goto('/tests/fixture.html'); await page.getByRole('button',{name:'Открыть проект',exact:true}).click();
  await page.getByRole('button',{name:/01. Северный ветер/}).click();
  const editor = page.getByRole('textbox',{name:'Текст документа'});
  await editor.fill('Новая редакция. Дракон вернулся.');
  await expect(page.getByRole('status')).toHaveText('Сохранено');
  await page.getByRole('button',{name:'Обновить историю'}).click();
  await page.getByRole('button',{name:/Версия 0/}).click();
  await expect(page.getByRole('dialog')).toContainText('Восстановить версию 0?');
  await page.getByRole('button',{name:'Восстановить версию',exact:true}).click();
  await expect(editor).toContainText('В день, когда море отступило');
  await expect(page.getByRole('status')).toHaveText('Сохранено');
  await page.screenshot({path:'test-results/workspace.png',fullPage:true});
});
test('new scene, folder navigation and literal search', async ({page}) => {
  await page.goto('/tests/fixture.html'); await page.getByRole('button',{name:'Открыть проект',exact:true}).click();
  await page.getByRole('button',{name:'Новая сцена',exact:true}).click();
  await page.getByLabel('Название',{exact:true}).fill('Вторая сцена'); await page.getByRole('button',{name:'Создать',exact:true}).click();
  await page.getByRole('textbox',{name:'Текст документа'}).fill('Ключевое слово: маяк.');
  await page.getByRole('textbox',{name:'Поиск по проекту'}).fill('маяк'); await page.getByRole('button',{name:'Найти',exact:true}).click();
  await expect(page.getByRole('navigation',{name:'Документы'})).toContainText('Вторая сцена');
  await page.getByRole('button',{name:'+ Папка',exact:true}).click();
  await page.getByLabel('Название',{exact:true}).fill('Часть II'); await page.getByRole('button',{name:'Создать',exact:true}).click();
  await page.getByRole('button',{name:'Часть II',exact:true}).click();
  await expect(page.getByText(/Здесь пока пусто/)).toBeVisible();
});
test('mobile preview has no horizontal overflow', async ({page}) => {
  await page.setViewportSize({width:390,height:844}); await page.goto('/');
  await expect(page.getByRole('heading',{level:1})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.screenshot({path:'test-results/mobile.png',fullPage:true});
});
