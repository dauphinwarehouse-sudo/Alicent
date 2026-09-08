import { test, expect, type Page } from "@playwright/test";
async function open(page: Page, suffix = "") {
  await page.goto(`/tests/fixture.html${suffix}`);
  await page.getByRole("button", {name: "Открыть проект", exact: true}).click();
  await page.getByRole("button", {name: "01. Северный ветер", exact: true}).click();
}
test("checkpoint flushes the draft, requires approval, restores and offers undo", async ({page}) => {
  await page.setViewportSize({width:1440,height:940}); await open(page);
  const editor = page.getByRole("textbox", {name:"Текст документа"});
  await editor.fill("Первый вариант — до правки.");
  await page.getByRole("button", {name:"Сохранность",exact:true}).click();
  await expect(editor).toHaveAttribute("contenteditable","false");
  await page.getByLabel("Название точки").fill("До редактуры");
  await page.getByRole("button", {name:"Создать точку",exact:true}).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("Контрольная точка создана");
  await page.getByRole("button", {name:"Закрыть сохранность"}).click();
  await editor.fill("Вторая редакция.");
  await page.getByRole("button", {name:"Сохранность",exact:true}).click();
  await page.getByRole("button", {name:/^До редактуры/}).click();
  await expect(page.getByRole("button", {name:"Восстановить тексты",exact:true})).toBeDisabled();
  await expect(page.locator(".checkpoint-preview")).toContainText("Изменится документов: 1");
  await page.screenshot({path:"test-results/ui/recovery-preview.png"});
  await page.getByLabel("Подтверждаю восстановление текстов всей рукописи").check();
  await page.screenshot({path:"test-results/ui/recovery-confirmation.png"});
  await page.getByRole("button", {name:"Восстановить тексты",exact:true}).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("Восстановлено документов: 1");
  await expect(page.getByRole("button", {name:/^Перед восстановлением:/})).toBeVisible();
  await page.getByRole("button", {name:"Закрыть сохранность"}).click();
  await expect(editor).toHaveText("Первый вариант — до правки.");
});
for (const theme of ["light","dark"] as const) test(`${theme}: backup dialog and copy result fit desktop`, async ({page}) => {
  await page.setViewportSize({width:1440,height:940}); await page.emulateMedia({colorScheme:theme}); await open(page);
  await page.getByRole("button", {name:"Сохранность",exact:true}).click();
  await page.getByRole("button", {name:"Создать резервную копию",exact:true}).click();
  await expect(page.getByRole("dialog").getByRole("status")).toContainText("Копия проверена и сохранена");
  const box = await page.getByRole("dialog").boundingBox();
  expect(box!.y).toBeGreaterThanOrEqual(0); expect(box!.y + box!.height).toBeLessThanOrEqual(940);
  await page.screenshot({path:`test-results/ui/recovery-${theme}.png`});
  await page.getByRole("button", {name:"Открыть из резервной копии"}).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("heading", {name:"Восстановленная рукопись"})).toBeVisible();
});
test("failed draft flush blocks backup and retains text",async ({page})=>{
  await open(page,"?save-error=1");
  const editor = page.getByRole("textbox",{name:"Текст документа"});
  await editor.fill("Сохраните этот черновик.");
  await page.getByRole("button",{name:"Сохранность",exact:true}).click();
  await page.getByRole("button",{name:"Создать резервную копию",exact:true}).click();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText("Тестовая ошибка сохранения");
  await expect(page.getByRole("dialog")).not.toContainText("Копия проверена");
  await page.screenshot({path:"test-results/ui/recovery-error.png"});
  await page.getByRole("button",{name:"Закрыть сохранность"}).click();
  await expect(editor).toHaveText("Сохраните этот черновик.");
});
test("390px: recovery modal stays in viewport and all actions are reachable",async ({page})=>{
  await page.setViewportSize({width:390,height:844}); await open(page);
  await page.getByRole("button",{name:"Сохранность",exact:true}).click();
  const box = await page.getByRole("dialog").boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0); expect(box!.x+box!.width).toBeLessThanOrEqual(390);
  expect(box!.y).toBeGreaterThanOrEqual(0); expect(box!.y+box!.height).toBeLessThanOrEqual(844);
  expect(await page.getByRole("dialog").evaluate(e=>e.scrollWidth<=e.clientWidth)).toBe(true);
  await page.screenshot({path:"test-results/ui/recovery-mobile.png"});
  await page.getByLabel("Название точки").fill("Мобильная точка");
  await page.getByRole("button",{name:"Создать точку",exact:true}).click();
  await expect(page.getByRole("button",{name:/^Мобильная точка/})).toBeVisible();
  await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0);
});
test("restore is available before a project is opened; backup is not",async ({page})=>{
  await page.goto("/tests/fixture.html"); await page.getByRole("button",{name:"Сохранность",exact:true}).click();
  await expect(page.getByRole("button",{name:"Создать резервную копию",exact:true})).toBeDisabled();
  await expect(page.getByRole("button",{name:"Открыть из резервной копии"})).toBeEnabled();
  await expect(page.getByLabel("Название точки")).toHaveCount(0);
});
