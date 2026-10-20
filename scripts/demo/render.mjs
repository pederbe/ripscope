// Render actual captured Vim screens. This never fabricates an editor result.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const imagesOnly = args[2] === '--images-only';
if (args.length < 2 || args.length > 3 || (args.length === 3 && !imagesOnly)) {
  throw new Error('usage: node scripts/demo/render.mjs CAPTURE-DIR NEW-OUTPUT-DIR [--images-only]');
}
const capture = resolve(args[0]), destination = resolve(args[1]);
mkdirSync(destination); // Never overwrite earlier captures or media.
const manifest = JSON.parse(readFileSync(join(capture, 'manifest.json'), 'utf8'));
if (manifest.format !== 1 || manifest.same_connection !== true || manifest.frames.length !== 4) throw new Error('Expected a verified four-frame Vim capture');
for (const [file, hash] of Object.entries(manifest.screen_sha256)) {
  if (!/^[0-9a-z-]+\.(txt|cursor)$/.test(file)) throw new Error('Invalid capture file name');
  const actual = createHash('sha256').update(readFileSync(join(capture, file))).digest('hex');
  if (actual !== hash) throw new Error(`Changed captured screen: ${file}`);
}
const observationBytes = readFileSync(join(capture, 'observation.json'));
if (createHash('sha256').update(observationBytes).digest('hex') !== manifest.observation_sha256) throw new Error('Changed capture observation');
const observation = JSON.parse(observationBytes);
if (observation.byte_identical_connection_records !== true || observation.before.quickfix.length !== 1 || observation.after.quickfix.length !== 2 || observation.after.quickfix[0].line !== 5 || observation.after.quickfix[1].line !== 8) throw new Error('Unexpected captured Vim navigation');
const expected = JSON.parse(readFileSync(resolve(import.meta.dirname, '../../tools/media/fixture/expected.json'), 'utf8'));
if (manifest.callers_before !== expected.before || manifest.callers_after !== expected.after) throw new Error('Capture rows no longer match shared fixture');

const frames = manifest.frames.map(frame => ({
  ...frame,
  lines: readFileSync(join(capture, frame.file), 'utf8').replace(/\n$/, '').split('\n'),
  cursor: readFileSync(join(capture, frame.file.replace('.txt', '.cursor')), 'utf8').trim().split(' ').map(Number),
}));
const last = frames.length;
// Quickfix rows exactly as the final captured screen shows them, checked against Vim's own records.
const quickfix = frames[last - 1].lines.map(line => line.match(/^ *\d+ (\S+\|\d+\| .*)$/)?.[1]).filter(Boolean);
if (quickfix.length !== observation.after.quickfix.length || quickfix.some((row, i) => !row.endsWith(`|${observation.after.quickfix[i].line}| ${observation.after.quickfix[i].text}`))) throw new Error('Final screen quickfix rows differ from the Vim records');
const added = observation.after.quickfix.filter(row => !observation.before.quickfix.some(old => old.line === row.line && old.text === row.text));

// To fit a 1200x675 frame, every screen drops the same number of Vim's empty "~" rows,
// and only the left 60 columns are shown. They hold all screen text apart from the
// status lines' cursor positions.
const filler = lines => {
  let best = { from: 0, length: 0 };
  for (let y = 0; y < lines.length; y++) {
    let end = y; while (end < lines.length && /^~ *$/.test(lines[end])) end++;
    if (end - y > best.length) best = { from: y, length: end - y };
    y = Math.max(y, end);
  }
  return best;
};
const fold = Math.min(...frames.map(frame => filler(frame.lines).length - 1));
const columns = 60;
const crop = line => {
  if (line.slice(columns).trim() && !/^ *\d+:\d+ *$/.test(line.slice(columns))) throw new Error(`Rendering would crop screen text: ${line}`);
  return line.slice(0, columns).trimEnd();
};

// Writes a captured screen into a <pre>, marking the cursor and any newly found caller rows.
function paint({ lines, cursor, added, columns, steps, active }) {
  const pre = document.getElementById('screen');
  pre.style.width = `calc(${columns}ch + 40px)`;
  pre.replaceChildren();
  lines.forEach((line, y) => {
    const row = added.some(suffix => line.endsWith(suffix)) ? document.createElement('span') : pre;
    if (row !== pre) { row.className = 'new'; pre.append(row); }
    if (y === cursor[1]) {
      const marker = document.createElement('span');
      marker.className = 'cursor'; marker.textContent = line[cursor[0]] || ' ';
      row.append(document.createTextNode(line.slice(0, cursor[0])), marker, document.createTextNode(line.slice(cursor[0] + 1)));
    } else row.append(document.createTextNode(line));
    if (y < lines.length - 1) pre.append(document.createTextNode('\n'));
  });
  document.getElementById('steps').replaceChildren(...steps.map((label, i) => {
    const step = document.createElement('div'), number = document.createElement('b');
    number.textContent = i + 1; step.append(number, label);
    if (i === active) step.className = 'on';
    return step;
  }));
  const screen = pre.getBoundingClientRect();
  return pre.scrollWidth > pre.clientWidth || pre.scrollHeight > pre.clientHeight || screen.bottom > 675 || screen.right > 1200;
}

const module = process.env.RIPSCOPE_PUPPETEER;
const { default: puppeteer } = await import(module ? pathToFileURL(resolve(module)) : 'puppeteer');
const mark = readFileSync(resolve(import.meta.dirname, '../../site/favicon.svg'), 'utf8').replace(/<rect[^>]*\/>/, '');
const browser = await puppeteer.launch({ headless: true, ...(process.env.BROWSER_PATH ? { executablePath: process.env.BROWSER_PATH } : {}) });
const browserVersion = await browser.version();
const page = await browser.newPage();
async function load(file, width, height) {
  await page.setViewport({ width, height });
  await page.setContent(readFileSync(join(import.meta.dirname, file), 'utf8'));
  await page.evaluateHandle('document.fonts.ready');
  await page.$eval('#brand', (element, mark) => { element.innerHTML = mark + '<span>Ripscope</span>'; }, mark);
}
try {
  await load('demo.html', 1200, 675);
  const steps = frames.map(frame => frame.label);
  for (const [index, frame] of frames.entries()) {
    const { from } = filler(frame.lines);
    const lines = [...frame.lines.slice(0, from + 1), ...frame.lines.slice(from + 1 + fold)].map(crop);
    const cursor = [frame.cursor[0], frame.cursor[1] > from ? frame.cursor[1] - fold : frame.cursor[1]];
    if (cursor[0] >= columns) throw new Error(`Rendering would crop the cursor in ${frame.file}`);
    if (await page.evaluate(paint, { lines, cursor, added: added.map(row => `|${row.line}| ${row.text}`), columns, steps, active: index })) throw new Error(`Terminal capture clipped in ${frame.file}`);
    await page.screenshot({ path: join(destination, `${index + 1}.png`) });
  }
  // The link preview shows the saved main.c around both callers and the final quickfix rows.
  await load('og.html', 1200, 630);
  const source = manifest.sources['main.c.after'].replace(/\n$/, '').split('\n');
  const lines = observation.after.quickfix.map(row => row.line);
  await page.evaluate(({ source, from, to, fresh, quickfix }) => {
    const code = document.getElementById('code');
    for (let n = from; n <= to; n++) {
      const row = document.createElement('span'); row.className = fresh.includes(n) ? 'row new' : 'row';
      const number = document.createElement('span'); number.className = 'ln'; number.textContent = n;
      row.append(number);
      for (const [token, string, type] of source[n - 1].matchAll(/("(?:[^"\\]|\\.)*")|\b(int|void|return|const|char)\b|[^"a-z]+|[a-z_]+/g)) {
        if (!string && !type) { row.append(document.createTextNode(token)); continue; }
        const span = document.createElement('span'); span.className = string ? 'st' : 'ty'; span.textContent = token; row.append(span);
      }
      code.append(row);
    }
    document.getElementById('rows').textContent = quickfix.join('\n');
    document.getElementById('count').textContent = `${quickfix.length} callers`;
  }, { source, from: Math.min(...lines) - 2, to: Math.max(...lines), fresh: added.map(row => row.line), quickfix });
  if (await page.evaluate(() => document.querySelector('main').scrollWidth > document.querySelector('main').clientWidth || document.querySelector('.panel').getBoundingClientRect().bottom > 630)) throw new Error('Social image panel is clipped');
  await page.screenshot({ path: join(destination, 'og.png') });
} finally { await browser.close(); }
copyFileSync(join(destination, `${last}.png`), join(destination, 'demo-poster.png'));

if (!imagesOnly) {
  // FFmpeg reads the actual screen snapshots with explicitly editorial holds.
  const concat = frames.map((frame, i) => `file '${i + 1}.png'\nduration ${frame.hold_seconds}`).join('\n') + `\nfile '${last}.png'\n`;
  const list = join(destination, 'frames.txt'); writeFileSync(list, concat);
  const duration = frames.reduce((total, frame) => total + frame.hold_seconds, 0);
  const ffmpeg = process.env.FFMPEG_PATH || 'ffmpeg';
  const encode = (extra, name) => execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'warning', '-y', '-f', 'concat', '-safe', '0', '-i', list, ...extra, '-t', String(duration), join(destination, name)], { stdio: 'inherit' });
  // H.264 needs an even height, so the video adds one background row below the 675-pixel frame.
  encode(['-vf', 'fps=10,pad=1200:676:0:0:color=0x0e1621', '-an', '-c:v', 'libx264', '-preset', 'medium', '-crf', '20', '-pix_fmt', 'yuv420p', '-movflags', '+faststart'], 'demo.mp4');
  encode(['-vf', 'fps=10,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=5', '-loop', '0'], 'demo.gif');
}
const generated = ['og.png', 'demo-poster.png', ...(imagesOnly ? [] : ['demo.mp4', 'demo.gif'])];
manifest.renderer = { browser: browserVersion, source_sha256: Object.fromEntries(['render.mjs', 'demo.html', 'og.html'].map(file => [file, createHash('sha256').update(readFileSync(join(import.meta.dirname, file))).digest('hex')])) };
manifest.rendered_sha256 = Object.fromEntries(generated.map(file => [file, createHash('sha256').update(readFileSync(join(destination, file))).digest('hex')]));
writeFileSync(join(destination, 'demo-manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
console.log(`Verified media written to ${destination}. No site assets were replaced.`);
