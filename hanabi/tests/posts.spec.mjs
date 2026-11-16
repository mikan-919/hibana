import { test, expect } from '@playwright/test';
import { makeServer } from './server.mjs';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

test('ビルドしたサーバーで入力拒否・作成・編集・再起動後の保存を確認する', async ({ page }) => {
  const server = await makeServer();
  const errors = [];
  const rejectedResources = [];
  const classes = new Set([...readFileSync(join(server.dir, 'index.html'), 'utf8').matchAll(/class="([^"]+)"/g)].flatMap(match => match[1].split(/\s+/)));
  const collectClasses = async () => {
    for (const value of await page.locator('[class]').evaluateAll(nodes => nodes.flatMap(node => [...node.classList]))) classes.add(value);
  };
  page.on('pageerror', error => errors.push(String(error)));
  page.on('console', message => {
    if (message.type() !== 'error') return;
    if (/^Failed to load resource:.*422/.test(message.text())) rejectedResources.push(message.text());
    else errors.push(message.text());
  });
  try {
    await page.goto(server.url);
    await expect(page.locator('#post-count')).toHaveText('0 件');
    await collectClasses();
    const title = page.getByLabel('タイトル', { exact: false });
    const body = page.getByLabel('本文', { exact: false });
    const submit = page.getByRole('button', { name: '投稿を保存', exact: true });
    for (const value of ['', 'x'.repeat(121)]) {
      await title.fill(value);
      await body.fill('本文');
      const rejected = page.waitForResponse(r => r.request().method() === 'POST' && r.url().endsWith('/api/posts'));
      await submit.click();
      expect((await rejected).status()).toBe(422);
      await expect(page.getByRole('status')).toContainText('title');
      await collectClasses();
      await expect(page.locator('#post-count')).toHaveText('0 件');
    }
    // フォームから送られた要求を型違反の JSON に変え、サーバーの拒否を画面まで戻す。
    await page.route('**/api/posts', async route => {
      if (route.request().method() === 'POST') await route.continue({ postData: JSON.stringify({ title: 42, body: '本文' }) });
      else await route.continue();
    });
    await title.fill('型違反を送る');
    const malformed = page.waitForResponse(r => r.request().method() === 'POST');
    await submit.click();
    expect((await malformed).status()).toBe(422);
    await expect(page.getByRole('status')).toContainText('title');
    await page.unroute('**/api/posts');
    await title.fill('取り消す入力');
    await page.getByRole('button', { name: '入力をクリア', exact: true }).press('Enter');
    await expect(title).toHaveValue('');
    await expect(body).toHaveValue('');
    const literal = '<script>window.hanabiInjected=true</script>\n本文🌸';
    await title.fill('共有契約から作る投稿');
    await body.fill(literal);
    const created = page.waitForResponse(r => r.request().method() === 'POST');
    await submit.click();
    expect((await created).status()).toBe(201);
    await expect(page.locator('#post-count')).toHaveText('1 件');
    await expect(page.locator('.post-body')).toHaveText(literal);
    await collectClasses();
    await expect(page.locator('article script')).toHaveCount(0);
    expect(await page.evaluate(() => window.hanabiInjected)).toBeUndefined();
    await page.getByRole('button', { name: '共有契約から作る投稿を編集', exact: true }).click();
    await expect(page.getByRole('heading', { name: '投稿を編集' })).toBeVisible();
    await title.fill('編集した投稿');
    await body.fill('更新した本文');
    const updated = page.waitForResponse(r => r.request().method() === 'PUT' && /\/api\/posts\/1$/.test(r.url()));
    await page.getByRole('button', { name: '変更を保存', exact: true }).click();
    expect((await updated).status()).toBe(200);
    await expect(page.locator('article h3')).toHaveText('編集した投稿');
    await expect(page.locator('.post-body')).toHaveText('更新した本文');
    await page.reload();
    await expect(page.locator('article h3')).toHaveText('編集した投稿');
    expect(server.persisted()).toEqual([{ id: 1, title: '編集した投稿', body: '更新した本文' }]);
    await server.restart();
    await page.reload();
    await expect(page.locator('article h3')).toHaveText('編集した投稿');
    await expect(page.locator('#post-count')).toHaveText('1 件');
    expect(errors).toEqual([]);
    expect(rejectedResources).toHaveLength(3);
    const styledClasses = await page.evaluate(() => {
      const names = new Set();
      const scan = rules => {
        for (const rule of rules) {
          if (rule.selectorText) for (const match of rule.selectorText.matchAll(/\.([A-Za-z][\w-]*)/g)) names.add(match[1]);
          if (rule.cssRules) scan(rule.cssRules);
        }
      };
      for (const sheet of document.styleSheets) scan(sheet.cssRules);
      return [...names];
    });
    expect(styledClasses.filter(value => !classes.has(value)), '未使用の CSS クラス').toEqual([]);
    expect([...classes].filter(value => !styledClasses.includes(value)), '定義の無い CSS クラス').toEqual([]);
  } finally { await server.close(); }
});
