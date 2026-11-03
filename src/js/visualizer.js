(function () {
    const BARS = 19
    const REF_RATE = 44100
    const LINES = 576
    const BAND_EDGES = [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13, 15, 16, 18, 20, 21, 23, 25, 27, 29, 31, 33, 36, 38, 41, 43, 46, 48, 51, 54, 57, 60, 64, 67, 71, 74, 78, 82, 86, 91, 95, 100, 105, 109, 115, 120, 126, 131, 137, 144, 150, 157, 164, 171, 178, 186, 194, 203, 211, 220, 230, 239, 250, 260, 271, 282, 294, 306, 319, 332, 345, 360, 374, 389, 576]
    const COLUMNS = 75
    const ROWS = 15
    const TICK = 0.0306
    const BAR_TICK = 1 / 48
    const BAR_FALLOFF = 32
    const UPDATE = 1 / 60
    const PEAK_SPEED = 3.0
    const PEAK_FALLOFF = 1.6
    const BAR_WIDTH = 3
    const GRID_WIDTH = BARS * (BAR_WIDTH + 1) - 1
    const GRID_HEIGHT = ROWS + 1
    const CLASSIC = ["24,132,8", "41,148,0", "49,156,8", "57,181,16", "50,190,16", "41,206,16", "148,222,33", "189,222,41",
        "214,181,33", "222,165,24", "198,123,8", "214,115,0", "214,102,0", "214,90,0", "206,41,16", "239,49,16"]

    function palette(name, color) {
        if (name === "classic") return { rows: CLASSIC.map((rgb) => `rgb(${rgb})`), peak: "rgb(150, 150, 150)" }
        const hex = /^#([0-9a-f]{6})$/i.test(color || "") && name === "accent" ? color.slice(1) : "ffffff"
        const rgb = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(", ")
        const rows = []
        for (let r = 0; r < GRID_HEIGHT; r++) rows.push(`rgba(${rgb}, ${(0.45 + 0.55 * r / (GRID_HEIGHT - 1)).toFixed(3)})`)
        return { rows, peak: `rgb(${rgb})` }
    }

    class Spectrum {
        constructor(canvas) {
            this.canvas = canvas
            this.context = null
            this.analysers = null
            this.input = null
            this.gain = null
            this.wired = false
            this.failed = false
            this.volume = 1
            this.balance = 0
            this.streams = new Map()
            this.colors = palette("classic")
            this.state = this.newState()
            this.canvas.width = GRID_WIDTH
            this.canvas.height = GRID_HEIGHT
            this.fit()
            window.addEventListener("resize", () => this.fit())
        }

        fit() {
            const ratio = window.devicePixelRatio || 1
            const scale = Math.max(1, Math.round(ratio))
            this.canvas.style.width = `${GRID_WIDTH * scale / ratio}px`
            this.canvas.style.height = `${GRID_HEIGHT * scale / ratio}px`
        }

        setColors(name, color) {
            this.colors = palette(name, color)
            if (this.frame) this.paint()
        }

        newState() {
            return { bars: new Array(BARS).fill(0), peaks: new Array(BARS).fill(0), speed: new Array(BARS).fill(0), tops: new Array(BARS).fill(0) }
        }

        prepare() {
            if (this.wired || this.failed) return
            const AudioContextClass = window.AudioContext || window.webkitAudioContext
            if (!AudioContextClass) {
                this.failed = true
                return
            }
            try {
                if (!this.context) {
                    try {
                        this.context = new AudioContextClass({ sampleRate: REF_RATE })
                    } catch (e) {
                        this.context = new AudioContextClass()
                    }
                }
                const running = this.context.state === "running" ? Promise.resolve() : this.context.resume()
                running.then(() => {
                    if (this.wired || this.context.state !== "running") return
                    this.input = this.context.createGain()
                    const splitter = this.context.createChannelSplitter(2)
                    this.analysers = [0, 1].map(() => {
                        const analyser = this.context.createAnalyser()
                        analyser.fftSize = 8192
                        return analyser
                    })
                    this.gain = this.context.createGain()
                    this.gain.gain.value = this.volume
                    this.panner = this.context.createStereoPanner()
                    this.panner.pan.value = this.balance
                    this.input.connect(this.gain)
                    this.gain.connect(this.panner)
                    this.panner.connect(this.context.destination)
                    this.input.connect(splitter)
                    splitter.connect(this.analysers[0], 0)
                    splitter.connect(this.analysers[1], 1)
                    this.wired = true
                    this.streams.forEach((stream, element) => this.route(element, stream))
                }).catch(() => { this.failed = true })
            } catch (e) {
                this.failed = true
            }
        }

        attach(element) {
            if (!this.streams.has(element)) this.streams.set(element, { level: 0 })
            const stream = this.streams.get(element)
            if (this.wired) this.route(element, stream)
            else element.volume = this.volume * stream.level
        }

        route(element, stream) {
            if (stream.node) return
            try {
                stream.source = this.context.createMediaElementSource(element)
                stream.node = this.context.createGain()
                stream.node.gain.value = stream.level
                stream.source.connect(stream.node)
                stream.node.connect(this.input)
                element.volume = 1
            } catch (e) {
                element.volume = this.volume * stream.level
            }
        }

        detach(element) {
            const stream = this.streams.get(element)
            if (!stream) return
            if (stream.node) {
                stream.source.disconnect()
                stream.node.disconnect()
            }
            this.streams.delete(element)
        }

        setLevel(element, level) {
            const stream = this.streams.get(element)
            if (!stream || stream.level === level) return
            stream.level = level
            if (stream.node) stream.node.gain.setTargetAtTime(level, this.context.currentTime, 0.015)
            else element.volume = this.volume * level
        }

        setVolume(volume) {
            this.volume = volume
            if (this.gain) this.gain.gain.value = volume
            this.streams.forEach((stream, element) => {
                if (!stream.node) element.volume = volume * stream.level
            })
        }

        setBalance(balance) {
            this.balance = balance
            if (this.panner) this.panner.pan.value = balance
        }

        channels() {
            if (!this.waves) return null
            const silent = (wave) => wave.every((v) => v === 0)
            const left = silent(this.waves[0])
            const right = silent(this.waves[1])
            if (left && right) return null
            return left || right ? 1 : 2
        }

        sampleRate() {
            return this.context ? this.context.sampleRate : 48000
        }

        latency() {
            if (!this.context) return 0
            return (this.context.outputLatency || 0) + (this.context.baseLatency || 0)
        }

        start() {
            this.prepare()
            if (this.frame) return
            this.last = performance.now()
            const tick = (now) => {
                this.frame = requestAnimationFrame(tick)
                const dt = Math.min((now - this.last) / 1000, 0.1)
                this.last = now
                this.draw(dt)
            }
            this.frame = requestAnimationFrame(tick)
        }

        setLoading(loading) {
            this.loading = loading
        }

        stop() {
            if (this.frame) cancelAnimationFrame(this.frame)
            this.frame = null
            this.loading = false
            this.loaderLevel = 0
            this.state = this.newState()
            this.pending = 0
            this.values = null
            this.canvas.getContext("2d").clearRect(0, 0, GRID_WIDTH, GRID_HEIGHT)
        }

        tables() {
            if (this.window) return
            const half = LINES / 2
            this.window = new Float64Array(2 * LINES)
            for (let n = 0; n < 2 * LINES; n++) this.window[n] = Math.sin(Math.PI * (n + 0.5) / (2 * LINES))
            this.preCos = new Float64Array(half)
            this.preSin = new Float64Array(half)
            this.postCos = new Float64Array(half)
            this.postSin = new Float64Array(half)
            for (let m = 0; m < half; m++) {
                this.preCos[m] = Math.cos(Math.PI * (m + 0.25) / LINES)
                this.preSin[m] = Math.sin(Math.PI * (m + 0.25) / LINES)
                this.postCos[m] = Math.cos(Math.PI * m / LINES)
                this.postSin[m] = Math.sin(Math.PI * m / LINES)
            }
            this.rev32 = new Uint8Array(32)
            for (let i = 0; i < 32; i++) {
                let r = 0
                for (let b = 0, v = i; b < 5; b++, v >>= 1) r = (r << 1) | (v & 1)
                this.rev32[i] = r
            }
            this.cos32 = new Float64Array(16)
            this.sin32 = new Float64Array(16)
            for (let j = 0; j < 16; j++) {
                this.cos32[j] = Math.cos(2 * Math.PI * j / 32)
                this.sin32[j] = Math.sin(2 * Math.PI * j / 32)
            }
            this.cos288 = new Float64Array(32 * 9)
            this.sin288 = new Float64Array(32 * 9)
            for (let k1 = 0; k1 < 32; k1++) {
                for (let n2 = 0; n2 < 9; n2++) {
                    this.cos288[k1 * 9 + n2] = Math.cos(2 * Math.PI * k1 * n2 / 288)
                    this.sin288[k1 * 9 + n2] = Math.sin(2 * Math.PI * k1 * n2 / 288)
                }
            }
            this.cos9 = new Float64Array(81)
            this.sin9 = new Float64Array(81)
            for (let i = 0; i < 81; i++) {
                this.cos9[i] = Math.cos(2 * Math.PI * i / 9)
                this.sin9[i] = Math.sin(2 * Math.PI * i / 9)
            }
            this.block = new Float64Array(2 * LINES)
            this.fold = new Float64Array(LINES)
            this.re = new Float64Array(half)
            this.im = new Float64Array(half)
            this.outRe = new Float64Array(half)
            this.outIm = new Float64Array(half)
            this.yr = new Float64Array(half)
            this.yi = new Float64Array(half)
            this.tr = new Float64Array(32)
            this.ti = new Float64Array(32)
            this.lines = [new Float64Array(LINES), new Float64Array(LINES)]
        }

        fft288() {
            const { re, im, yr, yi, tr, ti } = this
            for (let n2 = 0; n2 < 9; n2++) {
                for (let n1 = 0; n1 < 32; n1++) {
                    tr[this.rev32[n1]] = re[n1 * 9 + n2]
                    ti[this.rev32[n1]] = im[n1 * 9 + n2]
                }
                for (let size = 2; size <= 32; size <<= 1) {
                    const half = size >> 1
                    const step = 32 / size
                    for (let s = 0; s < 32; s += size) {
                        for (let k = 0; k < half; k++) {
                            const wr = this.cos32[k * step]
                            const wi = -this.sin32[k * step]
                            const a = s + k
                            const b = a + half
                            const xr = wr * tr[b] - wi * ti[b]
                            const xi = wr * ti[b] + wi * tr[b]
                            tr[b] = tr[a] - xr
                            ti[b] = ti[a] - xi
                            tr[a] += xr
                            ti[a] += xi
                        }
                    }
                }
                for (let k1 = 0; k1 < 32; k1++) {
                    const c = this.cos288[k1 * 9 + n2]
                    const s = this.sin288[k1 * 9 + n2]
                    yr[k1 * 9 + n2] = tr[k1] * c + ti[k1] * s
                    yi[k1 * 9 + n2] = ti[k1] * c - tr[k1] * s
                }
            }
            for (let k1 = 0; k1 < 32; k1++) {
                for (let k2 = 0; k2 < 9; k2++) {
                    let sr = 0
                    let si = 0
                    for (let n2 = 0; n2 < 9; n2++) {
                        const i = (n2 * k2) % 9
                        const c = this.cos9[i]
                        const s = this.sin9[i]
                        sr += yr[k1 * 9 + n2] * c + yi[k1 * 9 + n2] * s
                        si += yi[k1 * 9 + n2] * c - yr[k1 * 9 + n2] * s
                    }
                    this.outRe[k1 + 32 * k2] = sr
                    this.outIm[k1 + 32 * k2] = si
                }
            }
        }

        mdct(out) {
            const x = this.block
            const u = this.fold
            const half = LINES / 2
            for (let n = 0; n < half; n++) {
                u[n] = -x[3 * half - 1 - n] - x[3 * half + n]
                u[half + n] = x[n] - x[2 * half - 1 - n]
            }
            for (let m = 0; m < half; m++) {
                const ar = u[2 * m]
                const ai = u[LINES - 1 - 2 * m]
                this.re[m] = ar * this.preCos[m] + ai * this.preSin[m]
                this.im[m] = ai * this.preCos[m] - ar * this.preSin[m]
            }
            this.fft288()
            const scale = 2 / LINES
            for (let m = 0; m < half; m++) {
                const vr = this.outRe[m]
                const vi = this.outIm[m]
                out[2 * m] = (vr * this.postCos[m] + vi * this.postSin[m]) * scale
                out[LINES - 1 - 2 * m] = -(vi * this.postCos[m] - vr * this.postSin[m]) * scale
            }
        }

        readWaves() {
            const length = this.analysers[0].fftSize
            if (!this.waves || this.waves[0].length !== length) this.waves = [new Float32Array(length), new Float32Array(length)]
            this.analysers[0].getFloatTimeDomainData(this.waves[0])
            this.analysers[1].getFloatTimeDomainData(this.waves[1])
        }

        levels(back) {
            this.tables()
            const rate = this.sampleRate()
            const ratio = rate / REF_RATE
            const length = this.waves[0].length
            const span = (2 * LINES - 1) * ratio
            const end = Math.max(span, length - 1 - back * rate)
            const start = end - span
            const silent = (wave) => wave.every((v) => v === 0)
            const channels = silent(this.waves[1]) ? [0] : silent(this.waves[0]) ? [1] : [0, 1]
            channels.forEach((c) => {
                const wave = this.waves[c]
                for (let n = 0; n < 2 * LINES; n++) {
                    const t = start + n * ratio
                    const i = Math.floor(t)
                    const a = wave[i]
                    const b = wave[Math.min(i + 1, length - 1)]
                    this.block[n] = (a + (b - a) * (t - i)) * 32768 * this.window[n]
                }
                this.mdct(this.lines[c])
            })
            const columns = new Array(COLUMNS + 1).fill(0)
            for (let band = 0; band < COLUMNS; band++) {
                const from = BAND_EDGES[band]
                const to = BAND_EDGES[band + 1]
                let sum = 0
                for (let k = from; k < to; k++) {
                    sum += channels.length === 2 ? (Math.abs(this.lines[0][k]) + Math.abs(this.lines[1][k])) * 0.5 : Math.abs(this.lines[channels[0]][k])
                }
                columns[band] = Math.round(Math.min(255, Math.max(0, sum * (band * 12 / (to - from) + 1) * 0.000075)))
            }
            const levels = new Array(BARS)
            for (let bar = 0; bar < BARS; bar++) {
                const c = bar * 4
                levels[bar] = Math.min(ROWS, Math.trunc((columns[c] + columns[c + 1] + columns[c + 2] + columns[c + 3]) / 4))
            }
            return levels
        }

        step(state, levels, dt) {
            const ticks = dt / TICK
            const growth = Math.pow(PEAK_FALLOFF, ticks)
            for (let i = 0; i < BARS; i++) {
                const target = levels[i] * 16
                state.bars[i] = target >= state.bars[i] ? target : Math.max(target, state.bars[i] - BAR_FALLOFF * dt / BAR_TICK)
                const shown = state.bars[i] * 16
                if (state.peaks[i] <= shown) {
                    state.peaks[i] = shown
                    state.tops[i] = shown
                    state.speed[i] = PEAK_SPEED
                } else {
                    state.peaks[i] = Math.max(shown, state.peaks[i] - state.speed[i] * ticks)
                    state.speed[i] *= growth
                }
            }
        }

        draw(dt) {
            this.loaderTime = (this.loaderTime || 0) + dt
            this.loaderLevel = this.loading ? Math.min(1, (this.loaderLevel || 0) + dt / 0.2) : Math.max(0, (this.loaderLevel || 0) - dt / 0.5)
            if (this.wired) this.analyse(dt)
            this.paint()
        }

        analyse(dt) {
            this.pending = (this.pending || 0) + dt
            let updates = 0
            while (this.pending >= UPDATE * 0.75 && updates < 4) {
                this.pending -= UPDATE
                updates++
            }
            this.pending = Math.max(-UPDATE / 2, Math.min(UPDATE, this.pending))
            if (!updates) return
            this.readWaves()
            const latency = Math.max(0, Math.min(0.1, this.latency() - LINES / REF_RATE))
            for (let j = 0; j < updates; j++) {
                this.sinceValues = (this.sinceValues || TICK) + UPDATE
                if (!this.values || this.sinceValues >= TICK * 0.75) {
                    this.values = this.levels(latency + (updates - 1 - j) * UPDATE)
                    this.sinceValues = Math.max(0, this.sinceValues - TICK)
                }
                this.step(this.state, this.values, UPDATE)
            }
        }

        paint() {
            const ctx = this.canvas.getContext("2d")
            ctx.clearRect(0, 0, GRID_WIDTH, GRID_HEIGHT)
            const level = this.loaderLevel || 0
            if (level < 1) {
                ctx.globalAlpha = 1 - level
                this.paintBars(ctx)
            }
            if (level > 0) {
                ctx.globalAlpha = level
                this.paintLoader(ctx)
            }
            ctx.globalAlpha = 1
        }

        paintLoader(ctx) {
            const alpha = ctx.globalAlpha
            const head = ((this.loaderTime / 1.1) % 1) * (BARS + 6) - 3
            const middle = GRID_HEIGHT / 2
            for (let i = 0; i < BARS; i++) {
                const pulse = Math.max(0, 1 - Math.abs(i - head) / 3)
                const half = 1 + Math.round(pulse * 5)
                const x = i * (BAR_WIDTH + 1)
                ctx.globalAlpha = alpha * (0.3 + 0.7 * pulse)
                for (let r = middle - half; r < middle + half; r++) {
                    ctx.fillStyle = this.colors.rows[r]
                    ctx.fillRect(x, GRID_HEIGHT - 1 - r, BAR_WIDTH, 1)
                }
            }
            ctx.globalAlpha = alpha
        }

        paintBars(ctx) {
            for (let i = 0; i < BARS; i++) {
                const x = i * (BAR_WIDTH + 1)
                const rows = Math.min(ROWS, Math.round(this.state.bars[i] / 16))
                for (let r = 0; r < rows; r++) {
                    ctx.fillStyle = this.colors.rows[r]
                    ctx.fillRect(x, GRID_HEIGHT - 1 - r, BAR_WIDTH, 1)
                }
                const tops = this.state.tops[i]
                const peakRows = Math.round((tops - Math.floor((tops - this.state.peaks[i]) / 256) * 256) / 256)
                if (peakRows >= 1) {
                    ctx.fillStyle = this.colors.peak
                    ctx.fillRect(x, GRID_HEIGHT - 1 - Math.min(ROWS, peakRows), BAR_WIDTH, 1)
                }
            }
        }
    }

    window.Spectrum = Spectrum
})()
