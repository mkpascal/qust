#!/usr/bin/env node
// Compares a capture of the rebuilt UI with a reference render of the same state, size and scale.
//
//   node compare.mjs renders/player/stopped.dark@1.png my-capture.png [options]
//
//   --layout <file>     the state's layout file (default: inferred from the reference's path)
//   --scale <n>         device scale factor (default: from the reference's name, "@1.25")
//   --theme dark|light  (default: from the reference's name)
//   --tolerance <n>     per-channel difference (0-255) under which pixels count as equal (default 16)
//   --json <file>       also write the full report as JSON
//   --strict            no anti-aliasing allowance (a 1 px shift then shows up)
//   --quiet             only the summary line
//   --region <r>        compare only inside a box: an element's name from the layout file
//                       ("#playButton", "#cartridges>.cartridge[2]") or "x,y,w,h" in CSS px;
//                       text runs whose box centre lies outside it are left out (added for
//                       comparing single elements; without it nothing changes)
//
// Non-text pixels are compared one by one, with an anti-aliasing allowance (a pixel only counts
// as different if no pixel in the 3x3 neighbourhood of the other image matches it). Text runs
// (from the layout file) are masked out of that and compared the way glyphs from two rasterizers
// can be: by the bounding box of their ink, its coverage and its color. Writes
// <candidate>.diff.png next to the candidate: reference | candidate | difference (red: different
// pixels, yellow: anti-aliasing-only differences; text boxes outlined green when they match, red
// when they don't). Exit code 0 when everything is within tolerance, 1 otherwise, 2 on errors.

import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { PNG } = require('pngjs');
const ROOT = path.dirname(fileURLToPath(import.meta.url));

// Regions reported separately: elements with an id, and these classes.
const REGION_CLASSES = ['island', 'cartridge', 'cartridge-body', 'cartridge-eject', 'button', 'row', 'slider', 'setting', 'info-card', 'info-head', 'info-state', 'dialog', 'combo', 'shortcut', 'toggle-track', 'textbox', 'playlist-empty', 'playlist-status', 'channel', 'cartridge-ghost'];

function parseArgs(argv) {
    const options = { tolerance: 16 };
    const positional = [];
    for (let i = 0; i < argv.length; i++) {
        const arg = argv[i];
        if (arg === '--quiet') options.quiet = true;
        else if (arg === '--strict') options.strict = true;
        else if (arg.startsWith('--')) options[arg.slice(2)] = argv[++i];
        else positional.push(arg);
    }
    options.tolerance = Number(options.tolerance);
    [options.reference, options.candidate] = positional;
    return options;
}

function readPng(file) {
    const png = PNG.sync.read(fs.readFileSync(file));
    return { width: png.width, height: png.height, data: png.data };
}

// renders/<window>/<state>.<theme>@<scale>.png
function inferFromName(file) {
    const match = /([^\\/]+)[\\/]([^\\/]+)\.(dark|light)@([\d.]+)\.png$/.exec(file);
    if (!match) return {};
    const [, window, state, theme, scale] = match;
    return { window, state, theme, scale: Number(scale), layout: path.join(ROOT, 'layout', window, state + '.json') };
}

const themed = (value, theme) => (value && typeof value === 'object' && !Array.isArray(value) && 'dark' in value && 'light' in value ? value[theme] : value);

function intersect(a, b) {
    const x = Math.max(a[0], b[0]);
    const y = Math.max(a[1], b[1]);
    const right = Math.min(a[0] + a[2], b[0] + b[2]);
    const bottom = Math.min(a[1] + a[3], b[1] + b[3]);
    return [x, y, Math.max(0, right - x), Math.max(0, bottom - y)];
}

// A CSS px box as whole device pixels [x0, y0, x1, y1) inside the image.
function toDevice(box, scale, image, pad = 0) {
    const x0 = Math.max(0, Math.floor(box[0] * scale) - pad);
    const y0 = Math.max(0, Math.floor(box[1] * scale) - pad);
    const x1 = Math.min(image.width, Math.ceil((box[0] + box[2]) * scale) + pad);
    const y1 = Math.min(image.height, Math.ceil((box[1] + box[3]) * scale) + pad);
    return [x0, y0, x1, y1];
}

function pixel(image, x, y) {
    const i = (y * image.width + x) * 4;
    return [image.data[i], image.data[i + 1], image.data[i + 2]];
}

const distance = (a, b) => Math.max(Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1]), Math.abs(a[2] - b[2]));

function nearest(image, x, y, color) {
    let best = 255;
    for (let dy = -1; dy <= 1; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
            const nx = x + dx;
            const ny = y + dy;
            if (nx < 0 || ny < 0 || nx >= image.width || ny >= image.height) continue;
            best = Math.min(best, distance(color, pixel(image, nx, ny)));
            if (best === 0) return 0;
        }
    }
    return best;
}

// The ink of a text run inside a device-pixel box: pixels far enough from the background (the
// most common color on the box's edge).
function ink(image, [x0, y0, x1, y1]) {
    if (x1 - x0 < 2 || y1 - y0 < 2) return null;
    const counts = new Map();
    const edge = [];
    for (let x = x0; x < x1; x++) edge.push([x, y0], [x, y1 - 1]);
    for (let y = y0; y < y1; y++) edge.push([x0, y], [x1 - 1, y]);
    for (const [x, y] of edge) {
        const c = pixel(image, x, y);
        const key = (c[0] >> 3) + ',' + (c[1] >> 3) + ',' + (c[2] >> 3);
        const entry = counts.get(key) || { n: 0, sum: [0, 0, 0] };
        entry.n++;
        entry.sum = entry.sum.map((s, i) => s + c[i]);
        counts.set(key, entry);
    }
    const top = [...counts.values()].sort((a, b) => b.n - a.n)[0];
    const background = top.sum.map((s) => s / top.n);
    let contrast = 0;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) contrast = Math.max(contrast, distance(pixel(image, x, y), background));
    if (contrast < 20) return { background, contrast, count: 0 };
    const threshold = Math.max(18, contrast * 0.3);
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -1;
    let maxY = -1;
    let count = 0;
    const strong = [];
    for (let y = y0; y < y1; y++) {
        for (let x = x0; x < x1; x++) {
            const c = pixel(image, x, y);
            const d = distance(c, background);
            if (d < threshold) continue;
            count++;
            minX = Math.min(minX, x);
            minY = Math.min(minY, y);
            maxX = Math.max(maxX, x);
            maxY = Math.max(maxY, y);
            if (d >= contrast * 0.75) strong.push(c);
        }
    }
    const color = strong.length ? [0, 1, 2].map((i) => Math.round(strong.reduce((s, c) => s + c[i], 0) / strong.length)) : null;
    return { background: background.map(Math.round), contrast, count, box: count ? [minX, minY, maxX + 1, maxY + 1] : null, color };
}

export function compare(options) {
    const inferred = inferFromName(options.reference);
    const scale = Number(options.scale || inferred.scale || 1);
    const theme = options.theme || inferred.theme || 'dark';
    const layoutFile = options.layout || inferred.layout;
    const reference = readPng(options.reference);
    const candidate = readPng(options.candidate);
    if (reference.width !== candidate.width || reference.height !== candidate.height) {
        throw new Error(`size mismatch: reference ${reference.width}x${reference.height}, candidate ${candidate.width}x${candidate.height} (scale ${scale})`);
    }
    const layout = layoutFile && fs.existsSync(layoutFile) ? JSON.parse(fs.readFileSync(layoutFile, 'utf8')) : null;
    const scaled = layout && scale !== 1 && layout.scales && layout.scales[String(scale)];
    // Boxes at this scale: measured at it when the layout has them, else the scale-1 boxes scaled.
    const boxOf = (element) => (scaled && scaled.boxes[element.name]) || element.box;
    const elements = layout ? layout.elements.filter((element) => !element.onlyIn || element.onlyIn === theme) : [];
    let textRuns = layout ? (scaled ? scaled.text.map((run, index) => ({ ...layout.text[index], ...run })) : layout.text) : [];
    const { width, height } = reference;

    // --region: only pixels, regions and text runs inside this box (CSS px) count.
    let focus = null;
    if (options.region) {
        const parts = String(options.region).split(',').map(Number);
        if (parts.length === 4 && parts.every((n) => Number.isFinite(n))) focus = parts;
        else {
            const element = elements.find((e) => e.name === options.region);
            if (!element) throw new Error(`no element ${options.region} in the layout file`);
            focus = boxOf(element);
        }
        const centreInside = (rect) => {
            const cx = rect[0] + rect[2] / 2;
            const cy = rect[1] + rect[3] / 2;
            return cx >= focus[0] && cx <= focus[0] + focus[2] && cy >= focus[1] && cy <= focus[1] + focus[3];
        };
        textRuns = textRuns.map((run) => ({ ...run, rects: run.rects.filter(centreInside) })).filter((run) => run.rects.length);
    }
    const focusDevice = focus ? toDevice(focus, scale, reference) : [0, 0, width, height];
    const inFocus = (x, y) => x >= focusDevice[0] && x < focusDevice[2] && y >= focusDevice[1] && y < focusDevice[3];

    // Text mask
    const mask = new Uint8Array(width * height);
    const runs = [];
    for (const run of textRuns) {
        for (const rect of run.rects) {
            const visible = intersect(rect, run.clip);
            if (visible[2] < 1 || visible[3] < 1) continue;
            const device = toDevice(visible, scale, reference, Math.ceil(2 * scale));
            for (let y = device[1]; y < device[3]; y++) for (let x = device[0]; x < device[2]; x++) mask[y * width + x] = 1;
            runs.push({ element: run.element, text: run.text, rect: visible, device, color: themed(run.color, theme) });
        }
    }

    // Pixels
    const strict = new Uint8Array(width * height);
    const diff = new Uint8Array(width * height);
    let compared = 0;
    let strictCount = 0;
    let diffCount = 0;
    let sum = 0;
    for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
            const i = y * width + x;
            if (mask[i] || !inFocus(x, y)) continue;
            compared++;
            const a = pixel(reference, x, y);
            const b = pixel(candidate, x, y);
            const d = distance(a, b);
            sum += d;
            if (d <= options.tolerance) continue;
            strict[i] = 1;
            strictCount++;
            if (options.strict || (nearest(candidate, x, y, a) > options.tolerance && nearest(reference, x, y, b) > options.tolerance)) {
                diff[i] = Math.min(255, d);
                diffCount++;
            }
        }
    }

    // Regions
    const regions = [];
    for (const element of elements) {
        const classes = (themed(element.class, theme) || '').split(/\s+/);
        const ownId = /^#[^>]+$/.test(element.name);
        if (!ownId && !classes.some((name) => REGION_CLASSES.includes(name)) && element.tag !== 'canvas') continue;
        let box = intersect(boxOf(element), themed(element.visible, theme) || [0, 0, width / scale, height / scale]);
        if (focus) box = intersect(box, focus);
        if (box[2] < 1 || box[3] < 1) continue;
        const [x0, y0, x1, y1] = toDevice(box, scale, reference);
        let pixels = 0;
        let different = 0;
        let total = 0;
        for (let y = y0; y < y1; y++) {
            for (let x = x0; x < x1; x++) {
                const i = y * width + x;
                if (mask[i]) continue;
                pixels++;
                if (diff[i]) different++;
                total += distance(pixel(reference, x, y), pixel(candidate, x, y));
            }
        }
        if (!pixels) continue;
        regions.push({ name: element.name, box, pixels, different, ratio: different / pixels, meanDifference: total / pixels });
    }
    regions.sort((a, b) => b.ratio - a.ratio || b.meanDifference - a.meanDifference);

    // Text runs
    const position = Math.max(2, Math.round(1.5 * scale));
    const text = runs.map((run) => {
        const area = run.device;
        const a = ink(reference, area);
        const b = ink(candidate, area);
        const result = { element: run.element, text: run.text, box: run.rect };
        if (!a || !a.count) {
            result.status = b && b.count ? 'extra' : 'empty';
            result.ok = !(b && b.count > 4);
            return result;
        }
        if (!b || !b.count) return { ...result, status: 'missing', ok: false };
        const offsets = [0, 1, 2, 3].map((k) => b.box[k] - a.box[k]);
        const coverage = b.count / a.count;
        const colorDistance = a.color && b.color ? distance(a.color, b.color) : 0;
        const problems = [];
        if (offsets.some((offset) => Math.abs(offset) > position)) problems.push(`ink box off by [left ${offsets[0]}, top ${offsets[1]}, right ${offsets[2]}, bottom ${offsets[3]}] px`);
        if (coverage < 0.7 || coverage > 1.4) problems.push(`ink coverage ${(coverage * 100).toFixed(0)} % of the reference`);
        if (colorDistance > 40) problems.push(`color ${b.color} vs ${a.color}`);
        if (distance(a.background, b.background) > options.tolerance * 2) problems.push(`background ${b.background} vs ${a.background}`);
        return { ...result, ok: !problems.length, status: problems.length ? 'different' : 'ok', problems, reference: { box: a.box, color: a.color, count: a.count }, candidate: { box: b.box, color: b.color, count: b.count } };
    });

    // The picture: reference | candidate | difference
    const out = new PNG({ width: width * 3, height });
    for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
            const i = y * width + x;
            const a = pixel(reference, x, y);
            const b = pixel(candidate, x, y);
            const put = (panel, c) => {
                const o = (y * width * 3 + panel * width + x) * 4;
                out.data[o] = c[0];
                out.data[o + 1] = c[1];
                out.data[o + 2] = c[2];
                out.data[o + 3] = 255;
            };
            put(0, a);
            put(1, b);
            const gray = Math.round((a[0] * 0.3 + a[1] * 0.59 + a[2] * 0.11) * 0.35);
            let c = [gray, gray, gray];
            if (diff[i]) c = [255, Math.max(0, 80 - diff[i] / 4), 40];
            else if (strict[i]) c = [200, 180, 40];
            put(2, c);
        }
    }
    for (const run of text) {
        const [x0, y0, x1, y1] = toDevice(run.box, scale, reference, 1);
        const color = run.ok ? [40, 200, 90] : [255, 40, 40];
        for (let x = x0; x < x1; x++) {
            for (const y of [y0, y1 - 1]) {
                const o = (y * width * 3 + 2 * width + x) * 4;
                out.data.set(color, o);
            }
        }
        for (let y = y0; y < y1; y++) {
            for (const x of [x0, x1 - 1]) {
                const o = (y * width * 3 + 2 * width + x) * 4;
                out.data.set(color, o);
            }
        }
    }
    const diffFile = options.candidate.replace(/\.png$/i, '') + '.diff.png';
    fs.writeFileSync(diffFile, PNG.sync.write(out));

    const textFailed = text.filter((run) => !run.ok);
    const ratio = diffCount / Math.max(1, compared);
    const ok = ratio <= 0.001 && !textFailed.length;
    return {
        reference: options.reference,
        candidate: options.candidate,
        layout: layoutFile || null,
        scale,
        theme,
        size: [width, height],
        tolerance: options.tolerance,
        region: focus,
        ok,
        pixels: { compared, masked: width * height - compared, strictlyDifferent: strictCount, different: diffCount, ratio, meanDifference: sum / Math.max(1, compared) },
        text: { runs: text.length, ok: text.length - textFailed.length, failed: textFailed.length },
        regions,
        textRuns: text,
        diffImage: diffFile,
    };
}

function report(result, quiet) {
    const pct = (value) => (value * 100).toFixed(2) + ' %';
    console.log(`${result.ok ? 'MATCH' : 'DIFFERENT'}  pixels ${pct(result.pixels.ratio)} different (${result.pixels.different} of ${result.pixels.compared} non-text, ${result.pixels.strictlyDifferent} counting anti-aliasing, mean difference ${result.pixels.meanDifference.toFixed(2)}), text ${result.text.ok}/${result.text.runs} runs match`);
    if (quiet) return;
    const worst = result.regions.filter((region) => region.different > 0).slice(0, 15);
    if (worst.length) {
        console.log('\nRegions with different pixels (worst first):');
        for (const region of worst) console.log(`  ${pct(region.ratio).padStart(8)}  mean ${region.meanDifference.toFixed(1).padStart(5)}  ${region.name}  [${region.box.map((v) => Math.round(v)).join(', ')}]`);
    }
    const failed = result.textRuns.filter((run) => !run.ok);
    if (failed.length) {
        console.log('\nText runs that differ:');
        for (const run of failed.slice(0, 30)) console.log(`  ${run.status.padEnd(9)} ${JSON.stringify(run.text.slice(0, 40))} (${run.element})${run.problems ? ': ' + run.problems.join('; ') : ''}`);
    }
    if (!result.layout) console.log('\n(no layout file: no regions and no text comparison)');
    console.log(`\nDiff image: ${result.diffImage}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const options = parseArgs(process.argv.slice(2));
    if (!options.reference || !options.candidate) {
        console.error('usage: node compare.mjs <reference.png> <candidate.png> [--layout file] [--scale n] [--theme dark|light] [--tolerance n] [--json file] [--quiet]');
        process.exit(2);
    }
    try {
        const result = compare(options);
        report(result, options.quiet);
        if (options.json) fs.writeFileSync(options.json, JSON.stringify(result, null, 2) + '\n');
        process.exit(result.ok ? 0 : 1);
    } catch (error) {
        console.error(error.message);
        process.exit(2);
    }
}
