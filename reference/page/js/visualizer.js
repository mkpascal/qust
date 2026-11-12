(function () {
    const BARS = 19
    const ROWS = 15
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
            this.colors = palette("classic")
            this.frame = null
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
            if (this.frame) this.paint(this.frame)
        }

        paint(frame) {
            this.frame = new Uint8Array(frame)
            const ctx = this.canvas.getContext("2d")
            ctx.clearRect(0, 0, GRID_WIDTH, GRID_HEIGHT)
            const level = this.frame[0] / 255
            if (level < 1) {
                ctx.globalAlpha = 1 - level
                this.paintBars(ctx)
            }
            if (level > 0) this.paintLoader(ctx, level)
            ctx.globalAlpha = 1
        }

        paintLoader(ctx, level) {
            const middle = GRID_HEIGHT / 2
            for (let i = 0; i < BARS; i++) {
                const half = Math.min(middle, this.frame[3 + i * 4])
                const x = i * (BAR_WIDTH + 1)
                ctx.globalAlpha = level * this.frame[4 + i * 4] / 255
                for (let r = middle - half; r < middle + half; r++) {
                    ctx.fillStyle = this.colors.rows[r]
                    ctx.fillRect(x, GRID_HEIGHT - 1 - r, BAR_WIDTH, 1)
                }
            }
        }

        paintBars(ctx) {
            for (let i = 0; i < BARS; i++) {
                const x = i * (BAR_WIDTH + 1)
                const rows = Math.min(ROWS, this.frame[1 + i * 4])
                for (let r = 0; r < rows; r++) {
                    ctx.fillStyle = this.colors.rows[r]
                    ctx.fillRect(x, GRID_HEIGHT - 1 - r, BAR_WIDTH, 1)
                }
                const peak = Math.min(ROWS, this.frame[2 + i * 4])
                if (peak >= 1) {
                    ctx.fillStyle = this.colors.peak
                    ctx.fillRect(x, GRID_HEIGHT - 1 - peak, BAR_WIDTH, 1)
                }
            }
        }
    }

    window.Spectrum = Spectrum
})()
