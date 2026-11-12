// Generates tests/reference/frames.txt: what the page's analyzer (tests/reference/visualizer.js,
// the JS before the port, unchanged) draws for synthetic audio, frame by frame. tests/reference.rs
// regenerates the same audio, runs qust-spectrum and compares every frame byte for byte.
//
//   node tests/reference.mjs
//
// The signals use only +, -, *, /, %, floor, abs and 32-bit integer maths, which are exact in
// IEEE doubles, so Rust rebuilds them bit for bit (sin and cos would differ between libms).
// The frames are read from the real paintBars / paintLoader through a recording canvas.

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const source = fs.readFileSync(path.join(here, 'reference', 'visualizer.js'), 'utf8')

const WAVE_LEN = 8192
const BARS = 19

// Signals: a channel is a sum of components, rounded to Float32 like the analysers' data.

function wave(p) {
    const u = 2 * p - 1
    const y = 4 * u * (1 - Math.abs(u))
    return 0.225 * (y * Math.abs(y) - y) + y
}

function noise(n, seed) {
    let h = (n ^ seed) >>> 0
    h = Math.imul(h ^ (h >>> 16), 0x45d9f3b) >>> 0
    h = Math.imul(h ^ (h >>> 16), 0x45d9f3b) >>> 0
    h = (h ^ (h >>> 16)) >>> 0
    return h / 4294967296 * 2 - 1
}

function parse(spec) {
    if (spec === 'zero') return []
    return spec.split('+').map((part) => {
        const [kind, ...args] = part.trim().split(/\s+/)
        return { kind, args: args.map(Number) }
    })
}

function sample(components, n, rate) {
    let sum = 0
    for (const { kind, args } of components) {
        if (kind === 'sine') {
            const [f, a] = args
            sum += wave((n * f / rate) % 1) * a
        } else if (kind === 'sweep') {
            const [f0, f1, dur, a] = args
            const tt = (n / rate) % dur
            sum += wave((f0 * tt + (f1 - f0) * tt * tt / (2 * dur)) % 1) * a
        } else if (kind === 'noise') {
            const [seed, a] = args
            sum += noise(n, seed) * a
        } else throw new Error(kind)
    }
    return Math.fround(sum)
}

// A minimal page: a canvas that draws nothing, and an AudioContext whose analysers return the
// synthetic signals ending at `audio.pos`.

const audio = { pos: 0, rate: 44100, left: [], right: [] }

function analyser(channel) {
    return {
        fftSize: 2048,
        getFloatTimeDomainData(array) {
            const components = channel === 0 ? audio.left : audio.right
            const first = audio.pos - array.length
            for (let k = 0; k < array.length; k++) array[k] = sample(components, first + k, audio.rate)
        },
        connect() {},
        disconnect() {},
    }
}

class FakeContext {
    constructor() {
        this.sampleRate = audio.rate
        this.state = 'running'
        this.currentTime = 0
        // No output latency: Rust's reader already ends at the right sample, so `back` must be
        // only (updates - 1 - j) * UPDATE on both sides.
        this.outputLatency = 0
        this.baseLatency = 0
        this.destination = {}
        this.analysers = 0
    }
    node() {
        return { connect() {}, disconnect() {}, gain: { value: 1, setTargetAtTime() {} }, pan: { value: 0 } }
    }
    createGain() { return this.node() }
    createChannelSplitter() { return this.node() }
    createStereoPanner() { return this.node() }
    createAnalyser() { return analyser(this.analysers++) }
    resume() { return Promise.resolve() }
    close() { return Promise.resolve() }
}

const canvasContext = { clearRect() {}, fillRect() {}, fillStyle: '', globalAlpha: 1 }
const window = { devicePixelRatio: 1, addEventListener() {}, AudioContext: FakeContext }
new Function('window', 'performance', 'requestAnimationFrame', 'cancelAnimationFrame', source)(
    window, { now: () => 0 }, () => 1, () => {})
const Spectrum = window.Spectrum

function recorder() {
    const calls = []
    return {
        calls,
        fillStyle: '',
        globalAlpha: 1,
        fillRect(x, y, w, h) { calls.push({ x, y, w, h, style: this.fillStyle, alpha: this.globalAlpha }) },
    }
}

function check(ok, what) {
    if (!ok) throw new Error(what)
}

// As in Rust: 0..1 to a byte with 0 and 1 kept exact.
function unitByte(x) {
    if (x <= 0) return 0
    if (x >= 1) return 255
    return Math.min(254, Math.max(1, Math.round(x * 255)))
}

function frameOf(sp) {
    const frame = new Array(1 + BARS * 4).fill(0)
    frame[0] = unitByte(sp.loaderLevel || 0)
    if (!sp.off) {
        const rec = recorder()
        sp.paintBars(rec)
        const peak = sp.colors.peak
        const rows = new Array(BARS).fill(0)
        for (const call of rec.calls) {
            check(call.x % 4 === 0 && call.w === 3 && call.h === 1, 'bar geometry')
            const i = call.x / 4
            if (call.style === peak) {
                check(frame[1 + i * 4 + 1] === 0, 'one peak per bar')
                frame[1 + i * 4 + 1] = 15 - call.y
                check(frame[1 + i * 4 + 1] >= 1 && frame[1 + i * 4 + 1] <= 15, 'peak row')
            } else {
                check(call.y === 15 - rows[i], 'rows from the bottom')
                rows[i]++
            }
        }
        rows.forEach((r, i) => { frame[1 + i * 4] = r })
    }
    if ((sp.loaderLevel || 0) > 0) {
        const rec = recorder()
        rec.globalAlpha = 1
        sp.paintLoader(rec)
        const counts = new Array(BARS).fill(0)
        const glows = new Array(BARS).fill(null)
        for (const call of rec.calls) {
            const i = call.x / 4
            counts[i]++
            check(glows[i] === null || glows[i] === call.alpha, 'one alpha per loader block')
            glows[i] = call.alpha
        }
        for (let i = 0; i < BARS; i++) {
            const half = counts[i] / 2
            check(Number.isInteger(half) && half >= 1 && half <= 6, 'loader half')
            const ys = rec.calls.filter((c) => c.x === i * 4).map((c) => c.y)
            check(ys[0] === 15 - (8 - half) && ys[ys.length - 1] === 15 - (8 + half - 1), 'loader rows')
            frame[1 + i * 4 + 2] = half
            frame[1 + i * 4 + 3] = Math.round(glows[i] * 255)
        }
    }
    return frame
}

const hex = (bytes) => bytes.map((b) => b.toString(16).padStart(2, '0')).join('')

// FNV-1a over the doubles' bits, to compare the tables with Rust's without storing them.
function fnv(values) {
    let h = 0xcbf29ce484222325n
    const view = new DataView(new ArrayBuffer(8))
    for (const v of values) {
        view.setFloat64(0, v)
        h ^= view.getBigUint64(0)
        h = (h * 0x100000001b3n) & 0xffffffffffffffffn
    }
    return h.toString(16).padStart(16, '0')
}

// Deterministic frame timings.
let seed = 12345
function random() {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0
    return seed / 4294967296
}
const timings = {
    vsync60: () => 1 / 60 + (random() - 0.5) * 0.001,
    vsync144: () => 1 / 144 + (random() - 0.5) * 0.0005,
    vsync30: () => 1 / 30 + (random() - 0.5) * 0.002,
    // A busy page: mostly 60 Hz, sometimes a late frame, rarely a long stall (clamped to 0.1).
    janky: () => {
        const r = random()
        if (r < 0.75) return 1 / 60 + (random() - 0.5) * 0.004
        if (r < 0.9) return 0.02 + random() * 0.03
        if (r < 0.97) return random() * 0.012
        return 0.06 + random() * 0.3
    },
    // Every tick due for an update, whatever `pending` holds (see the unwired steps below).
    slow: () => 0.0215 + random() * 0.02,
    edges: (() => {
        const list = [1 / 60, 1 / 120, 0.0125, 0.0124, 0.0126, 0, 1 / 60, 0.1, 0.5, 1 / 30, 1 / 240, 0.05, 0.0667, 1 / 60]
        let i = 0
        return () => list[i++ % list.length]
    })(),
}

const out = []
out.push('# Generated by tests/reference.mjs from tests/reference/visualizer.js. Do not edit.')
let tables = null
let frames = 0

async function scenario(name, rate, body) {
    audio.rate = rate
    audio.pos = 200000
    const sp = new Spectrum({ width: 0, height: 0, style: {}, getContext: () => canvasContext })
    sp.prepare()
    await new Promise((resolve) => setTimeout(resolve, 0))
    check(sp.wired, 'wired')
    out.push(`scenario ${name} ${rate}`)
    let events = []
    let previous = null
    const s = {
        signal(left, right) {
            audio.left = parse(left)
            audio.right = parse(right)
            out.push(`left ${left}`)
            out.push(`right ${right}`)
        },
        loading(on) { sp.setLoading(on); events.push(on ? 'L1' : 'L0') },
        off(on) { sp.setColors(on ? 'off' : 'classic'); events.push(on ? 'O1' : 'O0') },
        reset() { sp.stop(); events.push('R') },
        wired(on) { sp.wired = on; events.push(on ? 'W1' : 'W0') },
        run(count, timing) {
            for (let n = 0; n < count; n++) {
                const dt = Math.fround(timing())
                sp.draw(Math.min(dt, 0.1))
                const frame = hex(frameOf(sp))
                // '=': the same frame as the step before, which keeps the file smaller.
                out.push(`step ${dt} ${audio.pos} ${events.length ? events.join(',') : '-'} ${frame === previous ? '=' : frame}`)
                previous = frame
                events = []
                audio.pos += Math.round(dt * rate)
                frames++
            }
        },
    }
    body(s)
    if (!tables && sp.window) {
        const growth = Math.pow(1.6, (1 / 60) / 0.0306)
        tables = ['window', 'preCos', 'preSin', 'postCos', 'postSin', 'cos32', 'sin32', 'cos288', 'sin288', 'cos9', 'sin9']
            .map((key) => `${key} ${fnv(sp[key])}`).concat(`growth ${fnv([growth])}`)
    }
}

const music = 'sine 110 0.3 + sine 440 0.25 + sine 1250 0.12 + sine 3300 0.06 + sine 9000 0.03 + noise 11 0.02'

await scenario('tones-44k', 44100, (s) => {
    s.signal('sine 440 0.5 + sine 3000 0.2', 'sine 440 0.5 + sine 8000 0.1')
    s.run(150, timings.vsync60)
    s.loading(true)
    s.run(40, timings.vsync60)
    s.loading(false)
    s.run(45, timings.vsync60)
    s.off(true)
    s.run(20, timings.vsync60)
    s.off(false)
    s.run(30, timings.vsync60)
    s.signal('zero', 'zero')
    s.run(90, timings.vsync60)
    s.signal(music, music)
    s.run(40, timings.vsync60)
    s.reset()
    s.run(40, timings.vsync60)
})

await scenario('sweep-48k', 48000, (s) => {
    s.signal('sweep 30 18000 4 0.6', 'sweep 30 18000 4 0.6')
    s.run(300, timings.vsync144)
    s.run(100, timings.janky)
})

await scenario('noise-44k', 44100, (s) => {
    s.signal('noise 1 0.3', 'noise 2 0.6')
    s.run(150, timings.janky)
    s.signal('noise 3 0.05 + sine 60 0.5', 'noise 4 0.02')
    s.run(100, timings.janky)
})

await scenario('one-channel-48k', 48000, (s) => {
    s.signal('sine 1000 0.7 + noise 3 0.05', 'zero')
    s.run(120, timings.vsync60)
    s.signal('zero', 'sine 250 0.4 + sweep 500 12000 2 0.3')
    s.run(120, timings.vsync60)
    s.signal('zero', 'zero')
    s.run(60, timings.vsync60)
})

await scenario('loud-44k', 44100, (s) => {
    s.signal('sine 60 0.6 + sine 5000 0.4 + noise 9 0.3', 'sine 61 0.6 + sine 7000 0.4 + noise 10 0.3')
    s.run(100, timings.vsync30)
    s.signal('zero', 'zero')
    s.run(60, timings.vsync30)
})

await scenario('quiet-48k', 48000, (s) => {
    s.signal('noise 5 0.0005 + sine 200 0.001', 'noise 6 0.0003')
    s.run(80, timings.vsync60)
    s.signal('noise 5 0.00002', 'zero')
    s.run(40, timings.vsync60)
})

await scenario('loader-44k', 44100, (s) => {
    s.signal(music, 'sine 220 0.4 + noise 8 0.05')
    s.loading(true)
    s.run(50, timings.vsync60)
    s.loading(false)
    s.run(10, timings.vsync60)
    s.loading(true)
    s.run(5, timings.vsync60)
    s.off(true)
    s.run(30, timings.janky)
    s.loading(false)
    s.run(40, timings.vsync60)
    s.off(false)
    s.loading(true)
    s.run(20, timings.vsync144)
    s.reset()
    s.run(30, timings.vsync60)
    s.loading(true)
    s.run(10, timings.vsync60)
    s.reset()
    s.loading(true)
    s.run(10, timings.vsync60)
    s.loading(false)
    s.run(40, timings.vsync60)
})

// Without audio the JS doesn't analyse at all. Rust learns that only from the reader, which it
// calls only on ticks due for an update, so these steps use ticks that are always due (see the
// Rust side: `pending` is restored when there is no audio).
await scenario('unwired-44k', 44100, (s) => {
    s.wired(false)
    s.signal(music, music)
    s.loading(true)
    s.run(40, timings.slow)
    s.wired(true)
    s.loading(false)
    s.run(100, timings.vsync60)
    s.wired(false)
    s.run(30, timings.slow)
    s.wired(true)
    s.run(60, timings.vsync60)
})

await scenario('edges-48k', 48000, (s) => {
    s.signal(music, music)
    s.run(140, timings.edges)
    s.signal('zero', 'zero')
    s.run(70, timings.edges)
})

out.splice(1, 0, `tables ${tables.join(' ')}`)
fs.writeFileSync(path.join(here, 'reference', 'frames.txt'), out.join('\n') + '\n')
console.log(`${frames} frames`)
