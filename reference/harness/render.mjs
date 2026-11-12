#!/usr/bin/env node
// Renders the state fixtures (reference/states/*.json) with the frozen page (reference/page) in a
// headless Edge, and measures them.
//
//   node harness/render.mjs                      every state, at its scales, both themes
//   node harness/render.mjs playing-fits stopped some states (names or globs: "drawer-*")
//   options: --scales 1,1.5   --themes dark   --all-scales   --jobs 4   --no-layout   --no-tokens
//            --gray   grayscale text (Chromium's --disable-lcd-text) into renders-gray/, no layout
//
// Writes renders/<window>/<state>.<theme>@<scale>.png, layout/<window>/<state>.json and
// layout/tokens.json. See README.md.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launch, Page } from './cdp.mjs';
import { makeFrame } from './frames.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PAGE_DIR = path.join(ROOT, 'page');
const STATES_DIR = path.join(ROOT, 'states');
const STUB = fs.readFileSync(path.join(ROOT, 'harness', 'stub.js'), 'utf8');
const COLLECT = fs.readFileSync(path.join(ROOT, 'harness', 'collect.js'), 'utf8');

export const ALL_SCALES = [1, 1.25, 1.5, 2];
export const THEMES = ['dark', 'light'];
// The Windows default accent (#0078d4) as main.rs's accent() reads it from the AccentPalette
// registry value: base = entry 3, light = entry 1 (Light 2), dark = entry 4 (Dark 1).
export const DEFAULT_ACCENT = { base: '#0078d4', light: '#4cc2ff', dark: '#0067c0' };

const PAGES = { player: 'index.html', settings: 'settings.html', info: 'info.html' };

function windowSize(fixture) {
    if (fixture.size) return fixture.size;
    if (fixture.window === 'player') return [860, fixture.drawer ? 587 : 267];
    if (fixture.window === 'settings') return [520, 850];
    if (fixture.window === 'info') return [580, 590];
    throw new Error(`unknown window ${fixture.window}`);
}

const READY = {
    player: "typeof player !== 'undefined' && player !== null && window.__REF__.hasChannel() && cartridgeParts.length === 6",
    settings: "document.getElementById('themeCombo').firstElementChild.textContent !== '' && document.getElementById('version').textContent !== ''",
    info: "document.getElementById('state').dataset.state !== undefined",
};

function merge(base, over) {
    if (Array.isArray(over) || typeof over !== 'object' || over === null) return over;
    const out = { ...base };
    for (const [key, value] of Object.entries(over)) {
        out[key] = key in out && typeof out[key] === 'object' && !Array.isArray(out[key]) && out[key] !== null ? merge(out[key], value) : value;
    }
    return out;
}

export function loadFixture(name) {
    const file = path.join(STATES_DIR, name + '.json');
    let fixture = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (fixture.extends) {
        const base = loadFixture(fixture.extends);
        delete base.name;
        delete base.times;
        delete base.description;
        fixture = merge(base, fixture);
        delete fixture.extends;
    }
    fixture.name = name;
    return fixture;
}

export function listStates() {
    return fs.readdirSync(STATES_DIR).filter((file) => file.endsWith('.json')).map((file) => file.slice(0, -5)).sort();
}

// A fixture with "times" is several states: <name>-t<ms> (or -p<percent>).
function expand(fixture) {
    if (!fixture.times) return [fixture];
    return fixture.times.map((time) => {
        const suffix = typeof time === 'string' && time.endsWith('%') ? `p${parseFloat(time)}` : `t${String(time).padStart(4, '0')}`;
        return { ...fixture, name: `${fixture.name}-${suffix}`, time, times: undefined, base: fixture.name };
    });
}

const nextFrame = 'new Promise((resolve) => { requestAnimationFrame(() => requestAnimationFrame(resolve)); setTimeout(resolve, 1000); })';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// DOM.getDocument resets the node ids, and with them every forced pseudo-state: the root is fetched
// once per page load and kept.
async function nodeFor(page, selector) {
    if (!page.root) page.root = (await page.send('DOM.getDocument', { depth: 0 })).root;
    const { nodeId } = await page.send('DOM.querySelector', { nodeId: page.root.nodeId, selector });
    if (!nodeId) throw new Error(`no element for ${selector}`);
    return nodeId;
}

async function centerOf(page, target) {
    if (typeof target === 'object' && target !== null && 'x' in target) return target;
    const spec = typeof target === 'string' ? { selector: target } : target;
    const rect = await page.evaluate(`(() => { const r = document.querySelector(${JSON.stringify(spec.selector)}).getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()`);
    const fx = spec.fx === undefined ? 0.5 : spec.fx;
    const fy = spec.fy === undefined ? 0.5 : spec.fy;
    return { x: rect[0] + rect[2] * fx + (spec.dx || 0), y: rect[1] + rect[3] * fy + (spec.dy || 0) };
}

async function runAction(page, action, fixture, scale) {
    const mouse = page.mouse;
    if ('eval' in action) {
        await page.evaluate(action.eval);
    } else if ('emit' in action) {
        await page.evaluate(`window.__REF__.emit(${JSON.stringify(action.emit)}, ${JSON.stringify(action.payload)})`);
    } else if ('frame' in action) {
        const frame = makeFrame(action.frame, { off: fixture.state && fixture.state.colors === 'off' });
        await page.evaluate(`window.__REF__.frame(${JSON.stringify([...frame])})`);
    } else if ('dragDrop' in action) {
        const payload = { ...action.dragDrop };
        if (payload.position) payload.position = { x: payload.position.x * scale, y: payload.position.y * scale };
        await page.evaluate(`window.__REF__.dragDrop(${JSON.stringify(payload)})`);
    } else if ('force' in action) {
        for (const [selector, states] of Object.entries(action.force)) {
            await page.send('CSS.forcePseudoState', { nodeId: await nodeFor(page, selector), forcedPseudoClasses: states });
            page.forced.push([selector, states]);
        }
    } else if ('move' in action || 'press' in action || 'dragTo' in action || 'release' in action || 'click' in action || 'contextmenu' in action) {
        const target = action.move || action.press || action.dragTo || action.click || action.contextmenu;
        if (target) {
            const to = await centerOf(page, target);
            const steps = action.steps || 1;
            const from = { ...mouse };
            for (let i = 1; i <= steps; i++) {
                mouse.x = from.x + (to.x - from.x) * (i / steps);
                mouse.y = from.y + (to.y - from.y) * (i / steps);
                await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: mouse.x, y: mouse.y, buttons: mouse.buttons, button: mouse.buttons ? 'left' : 'none' });
            }
        }
        const modifiers = action.modifiers || 0;
        if (action.press || action.click) {
            mouse.buttons = 1;
            await page.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: mouse.x, y: mouse.y, button: 'left', buttons: 1, clickCount: action.clickCount || 1, modifiers });
        }
        if (action.release || action.click) {
            mouse.buttons = 0;
            await page.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: mouse.x, y: mouse.y, button: 'left', buttons: 0, clickCount: action.clickCount || 1, modifiers });
        }
        if (action.contextmenu) {
            await page.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: mouse.x, y: mouse.y, button: 'right', buttons: 2, clickCount: 1, modifiers });
            await page.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: mouse.x, y: mouse.y, button: 'right', buttons: 0, clickCount: 1, modifiers });
        }
    } else if ('key' in action) {
        const { key, code = key, modifiers = 0, text } = typeof action.key === 'string' ? { key: action.key } : action.key;
        const keyCodes = { Escape: 27, Enter: 13, Tab: 9, Delete: 46, Backspace: 8, ArrowUp: 38, ArrowDown: 40, ArrowLeft: 37, ArrowRight: 39, Home: 36, End: 35, PageUp: 33, PageDown: 34 };
        const windowsVirtualKeyCode = keyCodes[key] || (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
        await page.send('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, code, modifiers, windowsVirtualKeyCode, text });
        await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, modifiers, windowsVirtualKeyCode });
    } else if ('scroll' in action) {
        await page.evaluate(`document.querySelector(${JSON.stringify(action.scroll)}).scrollTop = ${Number(action.top)}`);
    } else if ('wait' in action) {
        await sleep(action.wait);
    } else {
        throw new Error(`unknown action ${JSON.stringify(action)}`);
    }
    await page.evaluate(nextFrame);
}

// Every animation frozen `time` ms after it started (a number, or "N%" of the longest infinite
// animation's iteration). Without a time, infinite animations stop at their start and finite ones
// (the dialog opening, a dropped cartridge) at their end. Transitions always end.
const FREEZE = `(time, transitions) => {
    const all = document.getAnimations();
    const infinite = all.filter((a) => a.effect.getComputedTiming().iterations === Infinity);
    const longest = Math.max(0, ...infinite.map((a) => a.effect.getComputedTiming().duration));
    const t = typeof time === 'string' ? parseFloat(time) / 100 * longest : time;
    for (const a of all) {
        if (a instanceof CSSTransition && !(transitions && time !== null)) { a.finish(); continue; }
        const finite = a.effect.getComputedTiming().iterations !== Infinity;
        if (t === null || t === undefined) {
            if (finite) a.finish(); else { a.pause(); a.currentTime = 0; }
        } else {
            a.pause();
            a.currentTime = t;
        }
    }
    return all.length;
}`;

export async function renderOne(page, fixture, theme, scale, { layout = true } = {}) {
    const [width, height] = windowSize(fixture);
    await page.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: scale, mobile: false });
    await page.send('Emulation.setEmulatedMedia', {
        features: [
            { name: 'prefers-color-scheme', value: theme },
            { name: 'prefers-reduced-motion', value: 'no-preference' },
        ],
    });
    const injected = { ...fixture, accent: fixture.accent || DEFAULT_ACCENT };
    if (injected.window === 'player') injected.settings = { 'playlist-open': !!fixture.drawer, ...(fixture.settings || {}) };
    const { identifier } = await page.send('Page.addScriptToEvaluateOnNewDocument', { source: `window.__REF_FIXTURE__ = ${JSON.stringify(injected)};\n${STUB}` });
    const loaded = page.once('Page.loadEventFired');
    await page.send('Page.navigate', { url: pathToFileURL(path.join(PAGE_DIR, PAGES[fixture.window])).href });
    await loaded;
    await page.send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
    page.mouse = { x: 0, y: 0, buttons: 0 };
    page.root = null;
    page.forced = [];
    // No caret: its blink can't be frozen. (The real caret is a 1px bar in the text color.)
    await page.evaluate(`document.head.insertAdjacentHTML('beforeend', '<style id="__ref_caret">* { caret-color: transparent !important; }</style>')`);
    const started = Date.now();
    while (!(await page.evaluate(READY[fixture.window]))) {
        if (Date.now() - started > 5000) throw new Error(`${fixture.name}: the page never became ready`);
        await sleep(20);
    }
    await sleep(30);
    await page.evaluate(nextFrame);
    if (fixture.frame) {
        const frame = makeFrame(fixture.frame, { off: fixture.state && fixture.state.colors === 'off' });
        await page.evaluate(`window.__REF__.frame(${JSON.stringify([...frame])})`);
    }
    for (const action of fixture.actions || []) await runAction(page, action, fixture, scale);
    await sleep(30);
    await page.evaluate(nextFrame);
    await page.evaluate(`(${FREEZE})(${JSON.stringify(fixture.time === undefined ? null : fixture.time)}, ${!!fixture.transitions})`);
    await page.evaluate(nextFrame);
    const shot = await page.send('Page.captureScreenshot', { format: 'png', fromSurface: true, captureBeyondViewport: false });
    const png = Buffer.from(shot.data, 'base64');
    if (!layout) return { png };
    await page.evaluate(COLLECT);
    const measured = await page.evaluate('window.__refCollect()');
    measured.pseudo = await pseudoBoxes(page, measured.pseudo);
    measured.menus = await page.evaluate('window.__REF__.menus.map((menu) => menu.items)');
    measured.calls = await page.evaluate("window.__REF__.calls.filter((call) => !['get_data', 'player_attach', 'get_shortcuts', 'update_status', 'get_background', 'player_status', 'stream_info', 'pause_shortcuts'].includes(call.cmd))");
    return { png, measured };
}

// ::before / ::after boxes come from DevTools (scripts can't measure pseudo-elements).
async function pseudoBoxes(page, wanted) {
    if (!wanted.length) return [];
    const { root } = await page.send('DOM.getDocument', { depth: -1 });
    // That reset the forced pseudo-states: force them again before measuring.
    page.root = root;
    for (const [selector, states] of page.forced) {
        await page.send('CSS.forcePseudoState', { nodeId: await nodeFor(page, selector), forcedPseudoClasses: states });
    }
    if (page.forced.length) await page.evaluate('document.getAnimations().forEach((a) => { if (a instanceof CSSTransition) a.finish(); })');
    const found = [];
    const visit = async (node) => {
        for (const pseudo of node.pseudoElements || []) {
            if (pseudo.pseudoType !== 'before' && pseudo.pseudoType !== 'after') continue;
            const { object } = await page.send('DOM.resolveNode', { nodeId: node.nodeId });
            const call = await page.send('Runtime.callFunctionOn', {
                objectId: object.objectId,
                functionDeclaration: `function (pseudo) { return { name: window.__refName(this), style: window.__refStyle(this, pseudo) }; }`,
                arguments: [{ value: '::' + pseudo.pseudoType }],
                returnByValue: true,
            });
            const { name, style } = call.result.value;
            const full = name + '::' + pseudo.pseudoType;
            if (!wanted.includes(full)) continue;
            let box = null;
            try {
                const model = await page.send('DOM.getBoxModel', { nodeId: pseudo.nodeId });
                const quad = model.model.border;
                const xs = [quad[0], quad[2], quad[4], quad[6]];
                const ys = [quad[1], quad[3], quad[5], quad[7]];
                const r = (v) => Math.round(v * 100) / 100;
                box = [r(Math.min(...xs)), r(Math.min(...ys)), r(Math.max(...xs) - Math.min(...xs)), r(Math.max(...ys) - Math.min(...ys))];
            } catch (error) {
                box = null;
            }
            found.push({ name: full, box, style });
        }
        for (const child of node.children || []) await visit(child);
    };
    await visit(root);
    return found;
}

function scaleName(scale) {
    return String(scale);
}

// One line per element: easy to diff and to read.
function stringify(value) {
    return JSON.stringify(value, null, 1)
        .replace(/\n\s*/g, ' ')
        .replace(/("(?:elements|text|pseudo|animations|hidden|menus|calls)": \[) /g, '$1\n    ')
        .replace(/\}, \{ "name"/g, '},\n    { "name"')
        .replace(/\}, \{ "element"/g, '},\n    { "element"')
        .replace(/\}, \{ "target"/g, '},\n    { "target"');
}

export function layoutPath(window, state) {
    return path.join(ROOT, 'layout', window, state + '.json');
}

function writeLayout(fixture, results) {
    const file = layoutPath(fixture.window, fixture.name);
    let existing = {};
    try {
        existing = JSON.parse(fs.readFileSync(file, 'utf8'));
    } catch (error) {
        existing = {};
    }
    // What an earlier run measured, per theme, so rendering one theme keeps the other.
    const byTheme = {};
    for (const theme of existing.themes || []) byTheme[theme] = splitTheme(existing, theme);
    const scales = existing.scales || {};
    for (const { theme, scale, measured } of results) {
        if (!measured) continue;
        if (scale === 1) {
            byTheme[theme] = measured;
        } else {
            scales[scaleName(scale)] = {
                devicePixelRatio: measured.devicePixelRatio,
                boxes: Object.fromEntries(measured.elements.map((element) => [element.name, element.box])),
                text: measured.text.map(({ element, text, rects, clip }) => ({ element, text, rects, clip })),
            };
        }
    }
    const themes = THEMES.filter((theme) => byTheme[theme]);
    if (!themes.length) return;
    const merged = mergeThemes(themes.map((theme) => byTheme[theme]), themes);
    const out = {
        window: fixture.window,
        state: fixture.name,
        description: fixture.description || '',
        fixture: `states/${fixture.base || fixture.name}.json` + (fixture.time !== undefined ? ` (time ${fixture.time})` : ''),
        viewport: windowSize(fixture),
        themes,
        note: 'Boxes are [x, y, width, height] in CSS px at scale 1. A value that differs between the themes is { "dark": …, "light": … }. Colors are [r, g, b, a]. Omitted properties have their initial value (see layout/tokens.json "defaults").',
        ...merged,
        scales,
    };
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, stringify(out) + '\n');
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const isThemed = (value) => value !== null && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 2 && 'dark' in value && 'light' in value;

function mergeValue(values, themes) {
    if (values.every((value) => same(value, values[0]))) return values[0];
    // null: the property has its initial value (omitted) in that theme.
    return Object.fromEntries(themes.map((theme, index) => [theme, values[index] === undefined ? null : values[index]]));
}

function mergeObject(objects, themes) {
    const keys = [...new Set(objects.flatMap((object) => Object.keys(object || {})))];
    return Object.fromEntries(keys.map((key) => [key, mergeValue(objects.map((object) => (object || {})[key]), themes)]));
}

// One element list for both themes: style values (and anything else) that differ become
// { dark, light }. Lists are matched by name (elements, pseudo) or by position (text runs).
function mergeThemes(list, themes) {
    if (list.length === 1) {
        const [only] = list;
        return { devicePixelRatio: only.devicePixelRatio, elements: only.elements, pseudo: only.pseudo, text: only.text, hidden: only.hidden, scroll: only.scroll, animations: only.animations, menus: only.menus, calls: only.calls };
    }
    const byName = (items) => new Map(items.map((item) => [item.name, item]));
    const mergeNamed = (key) => {
        const maps = list.map((measured) => byName(measured[key]));
        const names = [...new Set(list.flatMap((measured) => measured[key].map((item) => item.name)))];
        return names.map((name) => {
            const items = maps.map((map) => map.get(name));
            if (items.some((item) => !item)) {
                const present = items.findIndex(Boolean);
                return { ...items[present], onlyIn: themes[present] };
            }
            const out = mergeObject(items.map(({ style, ...rest }) => rest), themes);
            out.style = mergeObject(items.map((item) => item.style), themes);
            return out;
        });
    };
    const text = list[0].text.map((run, index) => {
        const runs = list.map((measured) => measured.text[index]);
        if (runs.some((other) => !other || other.element !== run.element)) return run;
        return mergeObject(runs, themes);
    });
    return {
        devicePixelRatio: list[0].devicePixelRatio,
        elements: mergeNamed('elements'),
        pseudo: mergeNamed('pseudo'),
        text,
        hidden: list[0].hidden,
        scroll: list[0].scroll,
        animations: list[0].animations,
        menus: list[0].menus,
        calls: list[0].calls,
    };
}

// One theme's values back out of a merged layout file.
function splitTheme(merged, theme) {
    const pick = (value) => {
        if (isThemed(value)) return value[theme];
        if (Array.isArray(value)) return value.map(pick);
        if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, inner]) => [key, pick(inner)]));
        return value;
    };
    const keep = (items) => (items || []).filter((item) => !item.onlyIn || item.onlyIn === theme);
    return pick({
        devicePixelRatio: merged.devicePixelRatio,
        elements: keep(merged.elements),
        pseudo: keep(merged.pseudo),
        text: merged.text || [],
        hidden: merged.hidden || [],
        scroll: merged.scroll || [],
        animations: merged.animations || [],
        menus: merged.menus || [],
        calls: merged.calls || [],
    });
}

// `--gray` renders into renders-gray/: the same states with grayscale text antialiasing, which
// is what the real WebView2 window drew for most runs (README "Fidelity").
let RENDERS = 'renders';

export function renderPath(window, state, theme, scale) {
    return path.join(ROOT, RENDERS, window, `${state}.${theme}@${scaleName(scale)}.png`);
}

async function tokens(page) {
    const css = fs.readFileSync(path.join(PAGE_DIR, 'css', 'style.css'), 'utf8');
    const blocks = [...css.matchAll(/:root\s*\{([^}]*)\}/g)].map((match) => match[1]);
    const declared = {};
    blocks.forEach((block, index) => {
        for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
            declared[match[1]] = declared[match[1]] || {};
            declared[match[1]][index === 0 ? 'dark' : 'light'] = match[2].trim();
        }
    });
    const fixture = { ...loadFixture('stopped'), name: 'tokens', actions: [], frame: null };
    const out = {
        description: 'Every CSS variable in page/css/style.css: as declared (the light block overrides under prefers-color-scheme: light; tokens not redeclared there keep the dark value) and resolved in each theme with the default Windows accent injected (bridge.js sets --accent-base / --accent-light / --accent-dark from window.__QUST__.accent). Colors are [r, g, b, a] (0-255, alpha 0-1).',
        accent: DEFAULT_ACCENT,
        tokens: {},
        keyframes: {},
        fonts: {},
        defaults: 'Layout files omit a property when its computed value is the initial value: position static, opacity 1, transform none, box-shadow none, background transparent, borders with style none, outline none, padding and margin 0, gap normal, letter-spacing normal, text-transform none, no transition (0s) and no animation.',
    };
    for (const theme of THEMES) {
        await renderOne(page, fixture, theme, 1, { layout: false });
        const resolved = await page.evaluate(`(() => { const s = getComputedStyle(document.documentElement); return Object.fromEntries(${JSON.stringify(Object.keys(declared))}.map((name) => [name, s.getPropertyValue(name).trim()])); })()`);
        await page.evaluate(COLLECT);
        for (const [name, value] of Object.entries(resolved)) {
            out.tokens[name] = out.tokens[name] || { declared: declared[name] };
            const color = await page.evaluate(`(() => { const probe = document.createElement('i'); probe.style.color = ${JSON.stringify(value)}; document.body.appendChild(probe); const c = getComputedStyle(probe).color; probe.remove(); return probe.style.color ? c : null; })()`);
            const parsed = color ? await page.evaluate(`(() => { const m = /^rgba?\\(([^)]+)\\)$/.exec(${JSON.stringify(color)}); if (m) { const p = m[1].split(/[\\s,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1]; } const n = /^color\\(srgb ([^)]+)\\)$/.exec(${JSON.stringify(color)}); if (n) { const [rgb, a] = n[1].split('/'); const p = rgb.trim().split(/\\s+/).map(Number); return [Math.round(p[0]*255*1000)/1000, Math.round(p[1]*255*1000)/1000, Math.round(p[2]*255*1000)/1000, a === undefined ? 1 : Number(a)]; } return null; })()`) : null;
            const number = /^-?[\d.]+px$/.test(value) ? parseFloat(value) : undefined;
            out.tokens[name][theme] = parsed || (number !== undefined ? number : value);
        }
        // The fonts Chromium actually picked (proves the real Segoe UI Variable is used).
        const probes = { '#clockText': 'clock', '#title': 'title (dim)', '#station': 'station', '#kbps': 'specs number', '.cartridge-name': 'cartridge name', '#playlistButton': 'deck button' };
        for (const [selector, label] of Object.entries(probes)) {
            try {
                const { fonts } = await page.send('CSS.getPlatformFontsForNode', { nodeId: await nodeFor(page, selector) });
                out.fonts[`${label} (${selector})`] = fonts;
            } catch (error) {
                out.fonts[`${label} (${selector})`] = String(error.message);
            }
        }
    }
    out.keyframes = await page.evaluate(`(() => { const out = {}; for (const sheet of document.styleSheets) for (const rule of sheet.cssRules) if (rule instanceof CSSKeyframesRule) out[rule.name] = [...rule.cssRules].map((frame) => ({ keyText: frame.keyText, style: frame.style.cssText })); return out; })()`);
    fs.mkdirSync(path.join(ROOT, 'layout'), { recursive: true });
    fs.writeFileSync(path.join(ROOT, 'layout', 'tokens.json'), JSON.stringify(out, null, 2) + '\n');
}

function matches(name, patterns) {
    if (!patterns.length) return true;
    return patterns.some((pattern) => new RegExp('^' + pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.') + '$').test(name));
}

async function main() {
    const args = process.argv.slice(2);
    const option = (name) => {
        const index = args.indexOf(name);
        if (index < 0) return undefined;
        const value = args[index + 1];
        args.splice(index, 2);
        return value;
    };
    const flag = (name) => {
        const index = args.indexOf(name);
        if (index < 0) return false;
        args.splice(index, 1);
        return true;
    };
    const scalesOption = option('--scales');
    const themesOption = option('--themes');
    const jobs = Number(option('--jobs') || 4);
    const allScales = flag('--all-scales');
    const gray = flag('--gray');
    const noLayout = flag('--no-layout') || gray;
    const noTokens = flag('--no-tokens') || gray;
    if (gray) {
        RENDERS = 'renders-gray';
        process.env.QUST_REF_EDGE_ARGS = ((process.env.QUST_REF_EDGE_ARGS || '') + ' --disable-lcd-text').trim();
    }
    const patterns = args;

    const fixtures = listStates().filter((name) => !name.startsWith('_')).map(loadFixture).flatMap(expand).filter((fixture) => matches(fixture.name, patterns) || (fixture.base && matches(fixture.base, patterns)));
    if (!fixtures.length) throw new Error('no state matches');
    const work = [];
    for (const fixture of fixtures) {
        const scales = scalesOption ? scalesOption.split(',').map(Number) : allScales ? ALL_SCALES : fixture.scales || [1];
        const themes = themesOption ? themesOption.split(',') : fixture.themes || THEMES;
        for (const theme of themes) for (const scale of scales) work.push({ fixture, theme, scale });
    }

    const { browser, version, close } = await launch();
    console.log(`${version.product} â€” ${fixtures.length} states, ${work.length} renders, ${jobs} tabs`);
    const results = new Map();
    let done = 0;
    let failed = 0;
    try {
        const pages = await Promise.all(Array.from({ length: Math.min(jobs, work.length) }, () => Page.create(browser)));
        if (!noTokens && !patterns.length) await tokens(pages[0]);
        let next = 0;
        await Promise.all(pages.map(async (page) => {
            while (next < work.length) {
                const job = work[next++];
                const { fixture, theme, scale } = job;
                try {
                    const { png, measured } = await Promise.race([renderOne(page, fixture, theme, scale, { layout: !noLayout }), new Promise((_, reject) => setTimeout(() => reject(new Error('timed out after 60 s')), 60000))]);
                    const file = renderPath(fixture.window, fixture.name, theme, scale);
                    fs.mkdirSync(path.dirname(file), { recursive: true });
                    fs.writeFileSync(file, png);
                    if (!results.has(fixture)) results.set(fixture, []);
                    results.get(fixture).push({ theme, scale, measured });
                } catch (error) {
                    failed++;
                    console.error(`FAILED ${fixture.name} ${theme}@${scale}: ${error.message}`);
                }
                done++;
                if (done % 20 === 0 || done === work.length) console.log(`  ${done}/${work.length}`);
            }
        }));
    } finally {
        await close();
    }
    if (!noLayout) for (const [fixture, list] of results) writeLayout(fixture, list);
    console.log(failed ? `${failed} renders failed` : 'done');
    if (failed) process.exitCode = 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main().then(() => process.exit(process.exitCode || 0), (error) => {
        console.error(error);
        process.exit(1);
    });
}
