// Analyzer frames as qust_spectrum sends them (77 bytes): byte 0 is the loader's level (0-255);
// then for each of the 19 bars, 4 bytes: lit rows (0-15), peak row (0 = none, 1-15), loader
// half-height (1-6) and loader alpha (0-255). See crates/spectrum/src/lib.rs `frame()`.

export const BARS = 19;
export const FRAME_LEN = 1 + BARS * 4;

// Named bar shapes, for fixtures. Rows lit per bar (low to high frequencies) and peak rows.
export const SHAPES = {
    // A typical music frame: strong lows, a dip, lively mids, falling highs; peaks above.
    music: {
        bars: [11, 13, 14, 12, 9, 10, 11, 8, 9, 7, 8, 6, 7, 5, 4, 4, 3, 2, 1],
        peaks: [13, 14, 15, 14, 12, 11, 12, 11, 10, 9, 9, 8, 8, 7, 6, 5, 4, 3, 2],
    },
    // Everything at the top (bars clip at 15 rows, peaks at row 15).
    full: {
        bars: Array(BARS).fill(15),
        peaks: Array(BARS).fill(15),
    },
    // Quiet: low bars, peaks still falling (e.g. just after a pause).
    quiet: {
        bars: [3, 4, 4, 3, 2, 2, 3, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0],
        peaks: [9, 10, 11, 10, 8, 7, 8, 7, 6, 5, 5, 4, 4, 3, 3, 2, 2, 1, 0],
    },
    empty: { bars: Array(BARS).fill(0), peaks: Array(BARS).fill(0) },
};

// spec: { shape: name | { bars, peaks }, loader: 0..1 (level), loaderTime: seconds, off: bool }
// or a raw array of 77 numbers.
export function makeFrame(spec, { off = false } = {}) {
    if (Array.isArray(spec)) {
        if (spec.length !== FRAME_LEN) throw new Error(`a raw frame has ${FRAME_LEN} bytes`);
        return Uint8Array.from(spec);
    }
    const frame = new Uint8Array(FRAME_LEN);
    const shape = typeof spec.shape === 'string' ? SHAPES[spec.shape] : spec.shape || SHAPES.empty;
    if (!shape) throw new Error(`unknown frame shape ${spec.shape}`);
    const level = spec.loader || 0;
    frame[0] = Math.round(Math.min(1, Math.max(0, level)) * 255);
    if (!(off || spec.off)) {
        for (let i = 0; i < BARS; i++) {
            frame[1 + i * 4] = Math.min(15, shape.bars[i]);
            frame[2 + i * 4] = Math.min(15, shape.peaks[i]);
        }
    }
    if (level > 0) {
        // A pulse that sweeps left to right every 1.1 s (qust_spectrum).
        const head = (((spec.loaderTime || 0) / 1.1) % 1) * (BARS + 6) - 3;
        for (let i = 0; i < BARS; i++) {
            const pulse = Math.max(0, 1 - Math.abs(i - head) / 3);
            frame[3 + i * 4] = 1 + Math.round(pulse * 5);
            frame[4 + i * 4] = Math.round((0.3 + 0.7 * pulse) * 255);
        }
    }
    return frame;
}
