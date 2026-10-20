// Browser acceptance checks for the local landing page. No remote requests are made.
// Uses the optional diagram renderer's Puppeteer, or RIPSCOPE_PUPPETEER.
import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { after, before, test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

const modulePath = process.env.RIPSCOPE_PUPPETEER || resolve(import.meta.dirname,
  '../tools/site/node_modules/puppeteer/lib/puppeteer/puppeteer.js');
const { default: puppeteer } = await import(pathToFileURL(modulePath));
const site = resolve(process.env.RIPSCOPE_SITE_ROOT || resolve(import.meta.dirname, '../target/site'));
const screenshots = process.env.RIPSCOPE_SCREENSHOTS;
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml' };
const panels = ['script', 'windows', 'plugin', 'deb', 'rpm', 'source'].map(name => `panel-${name}`);
let browser, server, origin;

before(async () => {
  await readFile(resolve(site, 'index.html'));
  if (screenshots) await mkdir(screenshots, { recursive: true });
  server = createServer(async (request, response) => {
    try {
      const path = new URL(request.url, 'http://localhost').pathname;
      let file = resolve(site, `.${path === '/' ? '/index.html' : path}`);
      if (!file.startsWith(site + sep)) { response.writeHead(403).end(); return; }
      if (path === '/landing.js') await delay(600);
      if (!extname(file)) file += '.html';
      const data = await readFile(file);
      response.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream' });
      response.end(data);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await puppeteer.launch({ headless: true,
    ...(process.env.BROWSER_PATH ? { executablePath: process.env.BROWSER_PATH } : {}) });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise(resolve => server.close(resolve));
});
async function pageWith(options = {}) {
  const page = await browser.newPage();
  await page.setViewport(options.viewport || { width: 1280, height: 940 });
  if (options.noJS) await page.setJavaScriptEnabled(false);
  await page.setRequestInterception(true);
  page.on('request', request => {
    if (request.url().startsWith(origin) || request.url().startsWith('data:')) request.continue();
    else { request.abort(); assert.fail(`external runtime request: ${request.url()}`); }
  });
  return page;
}
async function visiblePanels(page) {
  return page.$$eval('.install-panel', elements => elements.filter(el => el.getBoundingClientRect().height)
    .map(el => el.id));
}

for (const [size, viewport] of [
  ['desktop', { width: 1280, height: 940 }],
  ['mobile', { width: 412, height: 915, isMobile: true, hasTouch: true }],
]) {
  test(`${size}: useful first frame, stable loading and Blueprint styling`, async t => {
    const page = await pageWith({ viewport });
    try {
      await page.evaluateOnNewDocument(() => {
        window.loading = { shifts: [], first: null };
        new PerformanceObserver(list => {
          for (const entry of list.getEntries()) if (!entry.hadRecentInput) window.loading.shifts.push(entry.value);
        }).observe({ type: 'layout-shift', buffered: true });
        new PerformanceObserver(list => {
          if (list.getEntries().some(entry => entry.name === 'first-contentful-paint')) {
            window.loading.first = { editor: document.querySelector('.editor-demo').getBoundingClientRect().height > 0,
              entrancesPrepared: [...document.querySelectorAll('[data-reveal]:not([data-revealed])')]
                .every(element => getComputedStyle(element).opacity === '0'),
              panels: [...document.querySelectorAll('.install-panel')].filter(el => el.getBoundingClientRect().height).map(el => el.id) };
          }
        }).observe({ type: 'paint', buffered: true });
      });
      await page.goto(origin, { waitUntil: 'load' });
      await delay(150);
      const loading = await page.evaluate(() => window.loading);
      assert.equal(loading.first?.editor, true);
      assert.equal(loading.first?.entrancesPrepared, true, 'entrances are prepared before the first paint');
      assert.deepEqual(loading.first?.panels, ['panel-script']);
      assert.ok(loading.shifts.reduce((a, b) => a + b, 0) < .1);
      t.diagnostic(JSON.stringify({ size, loading }));
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'no page overflow');
      assert.equal(await page.$eval('a.btn-primary', element => element.getAttribute('href')), '#install');
      assert.equal(await page.$$eval('main > section', elements => elements[1].id), 'install', 'installation follows the hero');
      const failures = await page.evaluate(() => {
        // Resolve modern CSS colors, including color-mix, to actual sRGB pixels.
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const context = canvas.getContext('2d', { willReadFrequently: true });
        const rgb = text => {
          context.clearRect(0, 0, 1, 1);
          context.fillStyle = text;
          context.fillRect(0, 0, 1, 1);
          const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
          return [r, g, b, a / 255];
        };
        const blend = (fore, back) => fore.slice(0, 3).map((value, i) => value * (fore[3] ?? 1) + back[i] * (1 - (fore[3] ?? 1)));
        const luminance = color => color.map(value => value / 255).map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4)
          .reduce((sum, value, i) => sum + value * [.2126, .7152, .0722][i], 0);
        function background(element) {
          if (!element) return [255, 255, 255];
          const color = rgb(getComputedStyle(element).backgroundColor);
          return blend(color, (color[3] ?? 1) < 1 ? background(element.parentElement) : [255, 255, 255]);
        }
        return [...document.querySelectorAll('body *')].filter(element => element.getBoundingClientRect().height > 0 &&
          [...element.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim()) &&
          !['SCRIPT', 'STYLE', 'NOSCRIPT', 'SVG'].includes(element.tagName)).flatMap(element => {
            const style = getComputedStyle(element);
            const bg = background(element);
            const fg = blend(rgb(style.color), bg);
            const [a, b] = [luminance(fg), luminance(bg)].sort((a, b) => b - a);
            const ratio = (a + .05) / (b + .05);
            const large = parseFloat(style.fontSize) >= 24 || (parseFloat(style.fontSize) >= 18.66 && parseInt(style.fontWeight) >= 700);
            return ratio + .01 < (large ? 3 : 4.5) ? [{ text: element.textContent.slice(0, 60), ratio }] : [];
          });
      });
      assert.deepEqual(failures, [], 'text contrast');
      if (screenshots) {
        await page.click('#motion-toggle');
        await page.screenshot({ path: resolve(screenshots, `landing-${size}.png`), fullPage: true });
        await page.screenshot({ path: resolve(screenshots, `landing-${size}-viewport.png`) });
        const performance = await page.$('#performance');
        await performance.screenshot({ path: resolve(screenshots, `performance-${size}.png`) });
      }
    } finally { await page.close(); }
  });
}

test('320px and 200% reflow keep commands inside focusable scroll regions', async () => {
  const page = await pageWith();
  try {
    for (const width of [320, 640]) {
      await page.setViewport({ width, height: 940 });
      await page.goto(origin, { waitUntil: 'load' });
      for (const panel of panels) {
        await page.click(`#tab-${panel.slice(6)}`);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${width}: ${panel}`);
        assert.ok(await page.$eval(`#${panel} code[tabindex="0"]`, element => getComputedStyle(element).overflowX === 'auto'));
      }
    }
  } finally { await page.close(); }
});

test('no JavaScript exposes every installation option and removes inactive controls', async () => {
  const page = await pageWith({ noJS: true, viewport: { width: 320, height: 940 } });
  try {
    await page.goto(origin, { waitUntil: 'load' });
    assert.deepEqual(await visiblePanels(page), panels);
    assert.ok(await page.$$eval('.install-tabs, .copy', elements => elements.every(el => el.getBoundingClientRect().height === 0)));
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.ok(await page.$eval('.editor-demo', element => element.getBoundingClientRect().height > 0));
    assert.ok(await page.$$eval('[data-reveal]', elements => elements.every(element => getComputedStyle(element).opacity === '1')));
  } finally { await page.close(); }
});

test('tabs support click, arrows, Home and End; skip link moves focus', async () => {
  const page = await pageWith();
  try {
    await page.goto(origin, { waitUntil: 'load' });
    await page.keyboard.press('Tab');
    assert.ok(await page.evaluate(() => document.activeElement.classList.contains('skip-link')));
    await page.keyboard.press('Enter');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'main-content');
    for (const panel of panels) {
      await page.click(`#tab-${panel.slice(6)}`);
      assert.deepEqual(await visiblePanels(page), [panel]);
    }
    await page.focus('#tab-source');
    for (const [key, panel] of [['ArrowRight', 'panel-script'], ['ArrowLeft', 'panel-source'], ['Home', 'panel-script'], ['End', 'panel-source']]) {
      await page.keyboard.press(key);
      assert.deepEqual(await visiblePanels(page), [panel]);
      assert.equal(await page.evaluate(() => document.activeElement.getAttribute('aria-controls')), panel);
    }
    assert.equal(await page.$$eval('[role="tab"][tabindex="0"]', elements => elements.length), 1);
  } finally { await page.close(); }
});

for (const outcome of ['success', 'missing', 'rejected']) {
  test(`copy command: clipboard ${outcome}`, async () => {
    const page = await pageWith();
    try {
      await page.evaluateOnNewDocument(outcome => Object.defineProperty(navigator, 'clipboard', {
        value: outcome === 'missing' ? undefined : { writeText(text) {
          window.copiedCommand = text;
          return outcome === 'rejected' ? Promise.reject(new Error('Denied')) : Promise.resolve();
        } },
      }), outcome);
      await page.goto(origin, { waitUntil: 'load' });
      await page.click('#tab-deb');
      await page.click('#panel-deb .copy');
      const command = await page.$eval('#command-deb', element => element.textContent);
      if (outcome === 'success') {
        await page.waitForFunction(() => document.getElementById('copy-status').textContent === 'Copied to the clipboard');
        assert.equal(await page.evaluate(() => window.copiedCommand), command);
      } else {
        await page.waitForFunction(() => document.getElementById('copy-status').textContent.includes('Command selected'));
        assert.equal(await page.evaluate(() => getSelection().toString()), command);
      }
    } finally { await page.close(); }
  });
}

test('reduced motion preserves the static demo, results, and installation controls', async () => {
  const page = await pageWith();
  try {
    await page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
    await page.goto(origin, { waitUntil: 'load' });
    assert.equal(await page.$eval('.scope-reticle', element => getComputedStyle(element).animationName), 'none');
    assert.equal(await page.$eval('.demo-scene-final .quickfix-row', element => element.textContent.trim()), 'main.c main 5 greet("world");\nmain.c helper 8 void helper(void ) { greet("x"); }');
    await page.click('#tab-windows');
    assert.deepEqual(await visiblePanels(page), ['panel-windows']);
  } finally { await page.close(); }
});

test('forced colors retains visible selection and focus indicators', async () => {
  const page = await pageWith();
  try {
    const client = await page.createCDPSession();
    await client.send('Emulation.setEmulatedMedia', { features: [{ name: 'forced-colors', value: 'active' }] });
    await page.goto(origin, { waitUntil: 'load' });
    await page.focus('#tab-script');
    const state = await page.$eval('#tab-script', element => ({
      focus: getComputedStyle(element).outlineStyle, selected: getComputedStyle(element).borderBottomStyle,
      art: getComputedStyle(document.querySelector('.scope-art')).display,
    }));
    assert.notEqual(state.focus, 'none');
    assert.notEqual(state.selected, 'none');
    assert.equal(state.art, 'none');
  } finally { await page.close(); }
});

test('structured data declares both core licenses and the Emacs license separately', async () => {
  const html = await readFile(resolve(site, 'index.html'), 'utf8');
  const data = JSON.parse(html.match(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/)[1]);
  assert.equal(data.license.length, 2);
  assert.ok(data.license.some(value => value.includes('mit')));
  assert.ok(data.license.some(value => value.includes('apache')));
  assert.match(data.hasPart.description, /GPL-2.0-or-later/);
});

async function animationTimes(page) {
  return page.$$eval('.demo-scene', scenes => scenes.map(scene => scene.getAnimations()[0]?.currentTime ?? null));
}
async function expectPause(page, action) {
  await action();
  await page.waitForFunction(() => !document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
  await page.waitForFunction(() => [...document.querySelectorAll('.demo-scene')].every(scene => scene.getAnimations().every(animation => animation.playState === 'paused' && !animation.pending)));
  const before = await animationTimes(page);
  await delay(180);
  const after = await animationTimes(page);
  assert.ok(after.every((time, i) => Math.abs(time - before[i]) < 5), 'animation time stays fixed while paused');
}
test('illustration uses the shared source-backed rows and static no-JS final scene', async () => {
  const expected = JSON.parse(await readFile(resolve(import.meta.dirname, '../tools/media/fixture/expected.json')));
  const page = await pageWith({ noJS: true });
  try {
    await page.goto(origin, { waitUntil: 'load' });
    assert.equal(await page.$eval('.demo-scene-first .quickfix-row', element => element.textContent.trim()), expected.before.trim());
    assert.equal(await page.$eval('.demo-scene-final .quickfix-row', element => element.textContent.trim()), expected.after.trim());
    assert.equal(await page.$eval('.demo-scene-final', element => getComputedStyle(element).opacity), '1');
    assert.equal(await page.$eval('.demo-controls', element => getComputedStyle(element).visibility), 'hidden');
  } finally { await page.close(); }
});
test('manual pause survives hover, focus and leaving/reentering the viewport', async () => {
  const page = await pageWith();
  try {
    await page.goto(origin, { waitUntil: 'load' });
    await page.mouse.move(0, 0);
    await page.$eval('.demo-visual', element => element.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
    const before = await animationTimes(page);
    await delay(200);
    const running = await animationTimes(page);
    assert.ok(running.every((time, i) => time > before[i] + 100), 'visible illustration advances');
    await expectPause(page, () => page.click('#demo-toggle'));
    await page.hover('.demo-visual');
    await page.mouse.move(0, 0);
    await page.focus('.demo-visual');
    await page.focus('#demo-toggle');
    await page.evaluate(() => scrollTo(0, document.body.scrollHeight));
    await page.$eval('.demo-visual', element => element.scrollIntoView({ block: 'center' }));
    assert.equal(await page.$eval('#demo-toggle', element => element.textContent), 'Resume illustration');
    assert.equal(await page.$eval('#editor-illustration', element => element.hasAttribute('data-demo-running')), false);
    // The button is outside the focus/hover pause region and can resume while focused.
    await page.focus('#demo-toggle');
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
    await expectPause(page, () => page.hover('.demo-visual'));
    await page.mouse.move(0, 0);
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
    await expectPause(page, () => page.focus('.demo-visual'));
    await page.focus('#demo-toggle');
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
    await expectPause(page, () => page.evaluate(() => scrollTo(0, document.body.scrollHeight)));
  } finally { await page.close(); }
});
test('motion changes only opacity and transform; reduced motion shows the final result', async () => {
  const page = await pageWith();
  try {
    await page.goto(origin, { waitUntil: 'load' });
    const properties = await page.$$eval('.demo-scene', scenes => scenes.flatMap(scene => scene.getAnimations()
      .flatMap(animation => animation.effect.getKeyframes().flatMap(frame => Object.keys(frame)))));
    assert.deepEqual([...new Set(properties.filter(key => !['offset', 'computedOffset', 'easing', 'composite'].includes(key)))].sort(), ['opacity', 'transform']);
    await page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-static'));
    assert.equal(await page.$eval('.demo-scene-final', element => getComputedStyle(element).opacity), '1');
    assert.equal(await page.$eval('.demo-scene-first', element => getComputedStyle(element).opacity), '0');
    assert.equal(await page.$eval('#demo-toggle', element => element.disabled), true);
  } finally { await page.close(); }
});

test('hidden document pauses the illustration and returning resumes it', async () => {
  const page = await pageWith();
  let other;
  try {
    await page.goto(origin, { waitUntil: 'load' });
    await page.mouse.move(0, 0);
    await page.$eval('.demo-visual', element => element.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
    other = await browser.newPage();
    await other.bringToFront();
    await page.waitForFunction(() => document.hidden && !document.getElementById('editor-illustration').hasAttribute('data-demo-running'), { polling: 50 });
    await page.waitForFunction(() => [...document.querySelectorAll('.demo-scene')].every(scene => scene.getAnimations().every(animation => animation.playState === 'paused' && !animation.pending)), { polling: 50 });
    const before = await animationTimes(page);
    await delay(180);
    const after = await animationTimes(page);
    assert.ok(after.every((time, i) => Math.abs(time - before[i]) < 5));
    await page.bringToFront();
    await page.waitForFunction(() => !document.hidden && document.getElementById('editor-illustration').hasAttribute('data-demo-running'));
  } finally { await other?.close(); await page.close(); }
});

test('page motion continues, pauses globally, and resumes in the update diagram', async () => {
  const page = await pageWith();
  const time = selector => page.$eval(selector, element => element.getAnimations()[0]?.currentTime);
  try {
    await page.goto(origin, { waitUntil: 'load' });
    await page.waitForFunction(() => document.querySelector('[data-motion="scan"]').hasAttribute('data-motion-active'));
    const before = await time('[data-motion="scan"]');
    await delay(240);
    assert.ok(await time('[data-motion="scan"]') > before + 100, 'scan keeps moving after entrance');
    assert.equal(await page.$eval('[data-aura]', element => getComputedStyle(element, '::before').animationPlayState), 'running');
    await page.click('#motion-toggle');
    await page.waitForFunction(() => document.querySelector('[data-motion="scan"]').getAnimations()[0].playState === 'paused');
    const paused = await time('[data-motion="scan"]');
    await delay(200);
    assert.ok(Math.abs(await time('[data-motion="scan"]') - paused) < 5);
    assert.equal(await page.$eval('#editor-illustration', element => element.hasAttribute('data-demo-running')), false);
    assert.equal(await page.$eval('[data-aura]', element => getComputedStyle(element, '::before').animationPlayState), 'paused');
    await page.$eval('#workflow-title', element => element.scrollIntoView({ block: 'center' }));
    assert.equal(await page.$eval('[data-motion="flow"]', element => element.hasAttribute('data-motion-active')), false);
    await page.click('#motion-toggle');
    await page.waitForFunction(() => document.querySelector('[data-motion="flow"]').hasAttribute('data-motion-active'));
    const flow = await time('[data-motion="flow"]');
    await delay(240);
    assert.ok(await time('[data-motion="flow"]') > flow + 100, 'update diagram has ongoing motion');
    await page.waitForFunction(() => document.querySelector('[data-motion="scan"]').getAnimations()[0].playState === 'paused');
  } finally { await page.close(); }
});

test('reduced motion disables every decorative loop and glow', async () => {
  const page = await pageWith();
  try {
    await page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
    await page.goto(origin, { waitUntil: 'load' });
    assert.ok(await page.$$eval('[data-motion]', elements => elements.every(element => element.getAnimations().length === 0)));
    assert.equal(await page.$eval('[data-aura]', element => getComputedStyle(element, '::before').animationName), 'none');
    assert.equal(await page.$eval('#motion-toggle', element => element.disabled), true);
    assert.ok(await page.$$eval('[data-reveal]', elements => elements.every(element => getComputedStyle(element).opacity === '1')));
  } finally { await page.close(); }
});

test('scroll entrances wait for a visible position and unfold at a readable pace', async t => {
  const page = await pageWith();
  let recorder;
  const state = () => page.$$eval('#performance [data-reveal]', elements => elements.map(element => ({
    opacity: Number(getComputedStyle(element).opacity),
    top: element.getBoundingClientRect().top,
    animations: element.getAnimations().map(animation => ({ time: animation.currentTime, state: animation.playState,
      duration: animation.effect.getTiming().duration, easing: animation.effect.getTiming().easing }))
  })));
  try {
    await page.goto(origin, { waitUntil: 'load' });
    const offscreen = await state();
    await page.$$eval('#performance [data-reveal]', elements => scrollTo(0,
      scrollY + Math.min(...elements.map(element => element.getBoundingClientRect().top)) - innerHeight + 8));
    await delay(160);
    const edge = await state();
    t.diagnostic(JSON.stringify({ offscreen, edge }));
    assert.ok(offscreen.every(element => element.opacity === 0), 'offscreen content is prepared before its entrance');
    assert.ok(edge.every(element => element.animations.length === 0), 'eight visible pixels do not consume the entrance below the viewport');
    if (screenshots && process.env.RIPSCOPE_RECORD_ENTRANCE) {
      recorder = await page.screencast({ path: resolve(screenshots, 'scroll-entrance.webm'), scale: .75 });
    }
    await page.$eval('#performance', element => element.scrollIntoView({ block: 'center' }));
    await page.waitForFunction(() => document.querySelector('#performance [data-reveal]').getAnimations().length > 0);
    await delay(400);
    const entering = await state();
    assert.ok(entering.every(element => element.opacity > 0 && element.opacity < .95), 'entrance is still visibly unfolding after 400ms');
    assert.ok(entering[0].opacity > entering[1].opacity + .05, 'related content enters in sequence');
    await page.waitForFunction(() => [...document.querySelectorAll('#performance [data-reveal]')].every(element =>
      element.getAnimations().length === 0 && getComputedStyle(element).opacity === '1'));
  } finally { await recorder?.stop(); await page.close(); }
});

test('default onboarding leads from installation to a first search and editor setup', async () => {
  const page = await pageWith();
  try {
    await page.goto(`${origin}/docs/getting-started/installation`, { waitUntil: 'load' });
    const install = await page.$eval('main', element => element.textContent);
    assert.match(install, /ripscope --version/);
    assert.doesNotMatch(install, /uninstall|RIPSCOPE_INSTALL_DIR|SQLite|attestation/i);
    if (screenshots) await page.screenshot({ path: resolve(screenshots, 'installation-desktop.png'), fullPage: true });
    await Promise.all([page.waitForNavigation({ waitUntil: 'load' }), page.click('main a[href*="quick-start"]')]);
    assert.match(await page.$eval('main', element => element.textContent), /ripscope -R -L -1 main/);
    await Promise.all([page.waitForNavigation({ waitUntil: 'load' }), page.click('main a[href*="editors/neovim"]')]);
    assert.match(await page.$eval('main', element => element.textContent), /RipscopeBuild/);
    assert.match(await page.$eval('main', element => element.textContent), /RipscopeFind g main/);
  } finally { await page.close(); }
});

test('docs sections stay visible while chapter submenus start collapsed', async () => {
  const page = await pageWith();
  const closedMenus = async () => {
    assert.equal(await page.$$eval('#mdbook-sidebar .part-title', titles =>
      titles.filter(title => title.getBoundingClientRect().height > 0).length), 6);
    assert.ok(await page.$eval('#mdbook-sidebar a[href$="getting-started/quick-start"]',
      link => link.getBoundingClientRect().height > 0));
    assert.ok(await page.$$eval('.chapter-fold-toggle', toggles =>
      toggles.every(toggle => toggle.getAttribute('aria-expanded') === 'false')));
  };
  try {
    await page.goto(`${origin}/docs/getting-started/installation`, { waitUntil: 'load' });
    await closedMenus();
    if (screenshots) await page.screenshot({ path: resolve(screenshots, 'docs-submenus-collapsed.png') });
    await page.focus('[aria-label="Toggle sections under Quick start"]');
    await page.keyboard.press('Enter');
    assert.equal(await page.$eval('[aria-label="Toggle sections under Quick start"]',
      toggle => toggle.getAttribute('aria-expanded')), 'true');
    await Promise.all([page.waitForNavigation({ waitUntil: 'load' }),
      page.click('#mdbook-sidebar a[href$="getting-started/example-project"]')]);
    assert.match(await page.$eval('main h1', element => element.textContent), /Example project/);
    await closedMenus();
    const benchmarks = await page.$('#mdbook-sidebar a[href$="reference/benchmarks"]');
    assert.equal(await benchmarks.evaluate(element => element.textContent), 'Benchmarks');
    await Promise.all([page.waitForNavigation({ waitUntil: 'load' }), benchmarks.click()]);
    assert.equal(await page.$eval('main h1', element => element.textContent), 'Benchmarks');
    await closedMenus();
  } finally { await page.close(); }
});
