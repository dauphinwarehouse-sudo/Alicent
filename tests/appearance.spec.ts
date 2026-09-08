import { test, expect } from "@playwright/test";

// Visual/regression fixtures only; no native filesystem or real user documents.
function contrast(first: string, second: string) {
  const luminance = (rgb: string) => {
    const values = rgb
      .match(/[\d.]+/g)!
      .slice(0, 3)
      .map(Number)
      .map((value) => {
        const channel = value / 255;
        return channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4;
      });
    return values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
  };
  const a = luminance(first),
    b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
for (const theme of ["light", "dark"] as const) {
  test(`${theme}: warm workspace, readable selection and fitting editor`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 940 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto("/tests/fixture.html");
    await page
      .getByRole("button", { name: "Открыть проект", exact: true })
      .click();
    await page
      .getByRole("button", { name: "01. Северный ветер", exact: true })
      .click();
    await expect(
      page.getByRole("textbox", { name: "Текст документа" }),
    ).toBeVisible();
    const colors = await page
      .locator(".document-list .selected")
      .evaluate((element) => {
        const style = getComputedStyle(element);
        return [style.color, style.backgroundColor];
      });
    expect(contrast(colors[0], colors[1])).toBeGreaterThanOrEqual(4.5);
    const syntaxColors = await page.locator(".cm-editor").evaluate((editor) => {
      const background = getComputedStyle(editor).backgroundColor;
      return [...editor.querySelectorAll(".cm-content span")]
        .filter((span) => span.textContent?.trim())
        .map((span) => [getComputedStyle(span).color, background]);
    });
    for (const [foreground, background] of syntaxColors) {
      expect(contrast(foreground, background)).toBeGreaterThanOrEqual(4.5);
    }

    const footer = await page.locator(".editor-footer").boundingBox();
    expect(footer!.y + footer!.height).toBeLessThanOrEqual(940);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(1440);
    await page.screenshot({
      path: `test-results/ui/${theme}-workspace.png`,
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Новая сцена", exact: true })
      .click();
    await expect(page.getByLabel("Название", { exact: true })).toBeFocused();
    await page.screenshot({
      path: `test-results/ui/${theme}-dialog.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(
      page.getByRole("textbox", { name: "Текст документа" }),
    ).toContainText("Элина");
  });
}

test("workshop links restore hidden panels without changing the document", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 940 });
  await page.goto("/tests/fixture.html");
  await page
    .getByRole("button", { name: "Открыть проект", exact: true })
    .click();
  await page
    .getByRole("button", { name: "01. Северный ветер", exact: true })
    .click();
  await page.getByRole("button", { name: "Фокус", exact: true }).click();
  await expect(page.locator("#project-tree")).toBeHidden();
  await page.screenshot({ path: "test-results/ui/focus.png", fullPage: true });
  await page.getByRole("link", { name: "История", exact: true }).click();
  await expect(page.locator("#version-history")).toBeVisible();
  await expect(page.locator("#project-tree")).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Текст документа" }),
  ).toContainText("Элина");
});

test("390px: manuscript, editor and history remain accessible", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/tests/fixture.html");
  await page
    .getByRole("button", { name: "Открыть проект", exact: true })
    .click();
  await page
    .getByRole("button", { name: "01. Северный ветер", exact: true })
    .click();
  for (const selector of ["#project-tree", "#writing", "#version-history"]) {
    await expect(page.locator(selector)).toBeVisible();
  }
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(390);
  await page.screenshot({
    path: "test-results/ui/mobile-workspace.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Новая сцена", exact: true }).click();
  const dialog = await page.getByRole("dialog").boundingBox();
  expect(dialog!.x).toBeGreaterThanOrEqual(0);
  expect(dialog!.x + dialog!.width).toBeLessThanOrEqual(390);
  await page.screenshot({
    path: "test-results/ui/mobile-dialog.png",
    fullPage: true,
  });
});

test("failed save remains visible and keeps the draft in the redesigned editor", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 940 });
  await page.goto("/tests/fixture.html?save-error=1");
  await page
    .getByRole("button", { name: "Открыть проект", exact: true })
    .click();
  await page
    .getByRole("button", { name: "01. Северный ветер", exact: true })
    .click();
  const editor = page.getByRole("textbox", { name: "Текст документа" });
  await editor.fill("Этот черновик нельзя потерять.");
  await expect(page.getByRole("status")).toHaveText("Не сохранено");
  await expect(page.getByRole("alert")).toContainText(
    "Тестовая ошибка сохранения",
  );
  await expect(editor).toHaveText("Этот черновик нельзя потерять.");
  await page.screenshot({
    path: "test-results/ui/save-error.png",
    fullPage: true,
  });
});
