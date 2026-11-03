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
    const SHADES = {
        classic: { bars: classicRows(), peak: "rgb(150, 150, 150)" },
        white: { bars: [[0, "rgba(255, 255, 255, 0.45)"], [1, "rgba(255, 255, 255, 1)"]], peak: "#ffffff" }
    }

    function classicRows() {
        const rows = ["24,132,8", "41,148,0", "49,156,8", "57,181,16", "50,190,16", "41,206,16", "148,222,33", "189,222,41",
            "214,181,33", "222,165,24", "198,123,8", "214,115,0", "214,102,0", "214,90,0", "206,41,16", "239,49,16"]
        const stops = []
        rows.forEach((rgb, r) => {
            stops.push([r / (ROWS + 1), `rgb(${rgb})`])
            stops.push([(r + 1) / (ROWS + 1), `rgb(${rgb})`])
        })
        return stops
    }

    class Spectrum {
        constructor(canvas, element) {
            this.canvas = canvas
            this.element = element
            this.context = null
            this.analysers = null
            this.gain = null
            this.wired = false
            this.failed = false
            this.theme = "classic"
            this.state = this.newState()
            this.canvas.addEventListener("dblclick", (event) => {
                event.preventDefault()
                this.theme = this.theme === "classic" ? "white" : "classic"
            })
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
                    const source = this.context.createMediaElementSource(this.element)
                    const splitter = this.context.createChannelSplitter(2)
                    this.analysers = [0, 1].map(() => {
                        const analyser = this.context.createAnalyser()
                        analyser.fftSize = 8192
                        return analyser
                    })
                    this.gain = this.context.createGain()
                    this.gain.gain.value = this.element.volume
                    this.element.volume = 1
                    source.connect(this.gain)
                    this.gain.connect(this.context.destination)
                    source.connect(splitter)
                    splitter.connect(this.analysers[0], 0)
                    splitter.connect(this.analysers[1], 1)
                    this.wired = true
                }).catch(() => { this.failed = true })
            } catch (e) {
                this.failed = true
            }
        }

        setVolume(volume) {
            if (this.wired && this.gain) {
                this.gain.gain.value = volume
                this.element.volume = 1
            } else {
                this.element.volume = volume
            }
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

        stop() {
            if (this.frame) cancelAnimationFrame(this.frame)
            this.frame = null
            this.state = this.newState()
            this.pending = 0
            this.values = null
            this.canvas.getContext("2d").clearRect(0, 0, this.canvas.width, this.canvas.height)
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
            if (!this.wired) return
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
            this.paint()
        }

        paint() {
            const canvas = this.canvas
            const state = this.state
            const ratio = window.devicePixelRatio || 1
            const width = Math.round(canvas.clientWidth * ratio)
            const height = Math.round(canvas.clientHeight * ratio)
            if (!width || !height) return
            if (canvas.width !== width || canvas.height !== height) {
                canvas.width = width
                canvas.height = height
                this.gradient = null
            }
            const ctx = canvas.getContext("2d")
            if (!this.gradient || this.gradientTheme !== this.theme) {
                this.gradient = ctx.createLinearGradient(0, height, 0, 0)
                SHADES[this.theme].bars.forEach(([stop, color]) => this.gradient.addColorStop(stop, color))
                this.gradientTheme = this.theme
            }
            ctx.clearRect(0, 0, width, height)
            const gap = Math.max(1, Math.round(2 * ratio))
            const barWidth = Math.max(1, Math.floor((width - gap * (BARS - 1)) / BARS))
            const offset = Math.floor((width - (barWidth * BARS + gap * (BARS - 1))) / 2)
            const peakHeight = Math.max(1, Math.round(2 * ratio))
            const row = height / (ROWS + 1)
            for (let i = 0; i < BARS; i++) {
                const x = offset + i * (barWidth + gap)
                const barHeight = Math.round(state.bars[i] / 16 * row)
                if (barHeight > 0) {
                    ctx.fillStyle = this.gradient
                    ctx.fillRect(x, height - barHeight, barWidth, barHeight)
                }
                const peakRows = (state.tops[i] - Math.floor((state.tops[i] - state.peaks[i]) / 256) * 256) / 256
                const peakY = height - Math.round(peakRows * row)
                if (peakRows >= 1) {
                    ctx.fillStyle = SHADES[this.theme].peak
                    ctx.fillRect(x, Math.max(0, peakY - peakHeight), barWidth, peakHeight)
                }
            }
        }
    }

    window.Spectrum = Spectrum
})()
