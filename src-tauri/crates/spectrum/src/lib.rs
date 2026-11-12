// Qust's spectrum analyzer: the analysis that used to run in the page (`src/js/visualizer.js`
// before the audio moved to Rust), ported number for number so the bars look exactly as before.
// It reads the pre-volume mix, runs a 576-line MDCT (the size of an MP3 granule), folds the lines
// into 75 columns and those into 19 bars, then animates bars and falling peaks. Each tick returns
// a 77-byte frame describing what the page paints on its 75×16 pixel canvas; colors stay in the
// page, since they come from CSS.
//
// The arithmetic deliberately mirrors the JS, operation by operation, in f64 (JS numbers are
// doubles), so that rounding lands on the same integers. Don't "simplify" an expression without
// rerunning the reference test (tests/reference.rs).

pub const BARS: usize = 19;
pub const FRAME_LEN: usize = 1 + BARS * 4;
/// Frames the analysis reads (the Web Audio analysers' fftSize).
pub const WAVE_LEN: usize = 8192;
/// The MDCT window is centered on what is heard now: the reader's last frame is this far ahead.
pub const LEAD_SECONDS: f32 = 576.0 / 44100.0;

const REF_RATE: f64 = 44100.0;
const LINES: usize = 576;
const HALF: usize = LINES / 2;
const BAND_EDGES: [usize; COLUMNS + 1] = [
    0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13, 15, 16, 18, 20, 21, 23, 25, 27, 29, 31, 33, 36, 38, 41, 43,
    46, 48, 51, 54, 57, 60, 64, 67, 71, 74, 78, 82, 86, 91, 95, 100, 105, 109, 115, 120, 126, 131,
    137, 144, 150, 157, 164, 171, 178, 186, 194, 203, 211, 220, 230, 239, 250, 260, 271, 282, 294,
    306, 319, 332, 345, 360, 374, 389, 576,
];
const COLUMNS: usize = 75;
const ROWS: f64 = 15.0;
const TICK: f64 = 0.0306;
const BAR_TICK: f64 = 1.0 / 48.0;
const BAR_FALLOFF: f64 = 32.0;
const UPDATE: f64 = 1.0 / 60.0;
const PEAK_SPEED: f64 = 3.0;
// The peaks fall faster each update by PEAK_FALLOFF (1.6) per TICK. The bars always step by UPDATE,
// so that's a constant: 1.6^(UPDATE / TICK) as V8's Math.pow rounds it (powf is 1 ulp higher).
const GROWTH: f64 = f64::from_bits(0x3ff4aafbc83207e8);

mod fdlibm;

// JS Math.round: halves go up, also for negatives (Rust's `round` goes away from zero).
// x - floor(x) is exact for every finite double, so the comparison is exact too.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

// 0..1 to a byte, keeping 0 and 1 exact: the page tests `level < 1` and `level > 0`.
fn unit_byte(x: f64) -> u8 {
    if x <= 0.0 {
        0
    } else if x >= 1.0 {
        255
    } else {
        js_round(x * 255.0).clamp(1.0, 254.0) as u8
    }
}

struct Tables {
    window: [f64; 2 * LINES],
    pre_cos: [f64; HALF],
    pre_sin: [f64; HALF],
    post_cos: [f64; HALF],
    post_sin: [f64; HALF],
    rev32: [usize; 32],
    cos32: [f64; 16],
    sin32: [f64; 16],
    cos288: [f64; 32 * 9],
    sin288: [f64; 32 * 9],
    cos9: [f64; 81],
    sin9: [f64; 81],
}

impl Tables {
    fn new() -> Box<Tables> {
        use std::f64::consts::PI;
        let mut t = Box::new(Tables {
            window: [0.0; 2 * LINES],
            pre_cos: [0.0; HALF],
            pre_sin: [0.0; HALF],
            post_cos: [0.0; HALF],
            post_sin: [0.0; HALF],
            rev32: [0; 32],
            cos32: [0.0; 16],
            sin32: [0.0; 16],
            cos288: [0.0; 32 * 9],
            sin288: [0.0; 32 * 9],
            cos9: [0.0; 81],
            sin9: [0.0; 81],
        });
        let lines = LINES as f64;
        for n in 0..2 * LINES {
            t.window[n] = fdlibm::sin(PI * (n as f64 + 0.5) / (2.0 * lines));
        }
        for m in 0..HALF {
            let m = m as f64;
            t.pre_cos[m as usize] = fdlibm::cos(PI * (m + 0.25) / lines);
            t.pre_sin[m as usize] = fdlibm::sin(PI * (m + 0.25) / lines);
            t.post_cos[m as usize] = fdlibm::cos(PI * m / lines);
            t.post_sin[m as usize] = fdlibm::sin(PI * m / lines);
        }
        for i in 0..32 {
            let mut r = 0;
            let mut v = i;
            for _ in 0..5 {
                r = (r << 1) | (v & 1);
                v >>= 1;
            }
            t.rev32[i] = r;
        }
        for j in 0..16 {
            t.cos32[j] = fdlibm::cos(2.0 * PI * j as f64 / 32.0);
            t.sin32[j] = fdlibm::sin(2.0 * PI * j as f64 / 32.0);
        }
        for k1 in 0..32 {
            for n2 in 0..9 {
                let angle = 2.0 * PI * k1 as f64 * n2 as f64 / 288.0;
                t.cos288[k1 * 9 + n2] = fdlibm::cos(angle);
                t.sin288[k1 * 9 + n2] = fdlibm::sin(angle);
            }
        }
        for i in 0..81 {
            t.cos9[i] = fdlibm::cos(2.0 * PI * i as f64 / 9.0);
            t.sin9[i] = fdlibm::sin(2.0 * PI * i as f64 / 9.0);
        }
        t
    }
}

// Scratch space for one MDCT, kept so a tick never allocates.
struct Work {
    block: [f64; 2 * LINES],
    fold: [f64; LINES],
    re: [f64; HALF],
    im: [f64; HALF],
    out_re: [f64; HALF],
    out_im: [f64; HALF],
    yr: [f64; HALF],
    yi: [f64; HALF],
    tr: [f64; 32],
    ti: [f64; 32],
    lines: [[f64; LINES]; 2],
}

#[derive(Clone, Copy)]
struct Bars {
    bars: [f64; BARS],
    peaks: [f64; BARS],
    speed: [f64; BARS],
    tops: [f64; BARS],
}

impl Bars {
    const NEW: Bars = Bars { bars: [0.0; BARS], peaks: [0.0; BARS], speed: [0.0; BARS], tops: [0.0; BARS] };
}

pub struct Spectrum {
    tables: Box<Tables>,
    work: Box<Work>,
    waves: [Box<[f32]>; 2],
    state: Bars,
    // The bar levels from the last analysis (0..=15), reused between analyses: they are computed
    // every TICK (30.6 ms) while the bars move every UPDATE (1/60 s).
    values: Option<[f64; BARS]>,
    since_values: f64,
    pending: f64,
    off: bool,
    loading: bool,
    loader_level: f64,
    loader_time: f64,
}

impl Default for Spectrum {
    fn default() -> Self {
        Spectrum::new()
    }
}

impl Spectrum {
    pub fn new() -> Spectrum {
        Spectrum {
            tables: Tables::new(),
            work: Box::new(Work {
                block: [0.0; 2 * LINES],
                fold: [0.0; LINES],
                re: [0.0; HALF],
                im: [0.0; HALF],
                out_re: [0.0; HALF],
                out_im: [0.0; HALF],
                yr: [0.0; HALF],
                yi: [0.0; HALF],
                tr: [0.0; 32],
                ti: [0.0; 32],
                lines: [[0.0; LINES]; 2],
            }),
            waves: [vec![0.0; WAVE_LEN].into_boxed_slice(), vec![0.0; WAVE_LEN].into_boxed_slice()],
            state: Bars::NEW,
            values: None,
            since_values: 0.0,
            pending: 0.0,
            off: false,
            loading: false,
            loader_level: 0.0,
            loader_time: 0.0,
        }
    }

    /// "Off" colors: no bars and no analysis; the loader still shows. Flipping it starts the bars
    /// afresh.
    pub fn set_off(&mut self, off: bool) {
        if off != self.off {
            self.state = Bars::NEW;
        }
        self.off = off;
    }

    /// Connecting or buffering: the loader fades in over 0.2 s, and out over 0.5 s.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Stopping: bars, peaks and loader cleared. As in the JS, the loader's phase and the analysis
    /// cadence carry on.
    pub fn reset(&mut self) {
        self.loading = false;
        self.loader_level = 0.0;
        self.state = Bars::NEW;
        self.pending = 0.0;
        self.values = None;
    }

    /// Nothing lit and nothing fading: with no new audio, every frame from now on is blank, so the
    /// caller may stop ticking until something changes.
    pub fn idle(&self) -> bool {
        let zero = |values: &[f64; BARS]| values.iter().all(|&v| v == 0.0);
        !self.loading
            && self.loader_level <= 0.0
            && zero(&self.state.bars)
            && zero(&self.state.peaks)
            && zero(&self.state.tops)
    }

    /// Advances by `dt` seconds (at most 0.1, as the page's animation frames were) and returns the
    /// frame to draw. `read` is called at most once, only when the bars are due to move: it fills
    /// both slices (WAVE_LEN each) with the latest frames, ending at what is heard LEAD_SECONDS from
    /// now, and returns their sample rate, or None when there is no audio.
    pub fn tick(&mut self, dt: f32, read: &mut dyn FnMut(&mut [f32], &mut [f32]) -> Option<f32>) -> [u8; FRAME_LEN] {
        let dt = (dt as f64).min(0.1);
        self.loader_time += dt;
        self.loader_level = if self.loading {
            (self.loader_level + dt / 0.2).min(1.0)
        } else {
            (self.loader_level - dt / 0.5).max(0.0)
        };
        if !self.off {
            self.analyse(dt, read);
        }
        self.frame()
    }

    // Moves the bars at a steady 60 updates per second whatever the caller's rate: up to 4
    // updates per tick catch up after a slow frame, and each lags `back` behind the latest audio.
    fn analyse(&mut self, dt: f64, read: &mut dyn FnMut(&mut [f32], &mut [f32]) -> Option<f32>) {
        let before = self.pending;
        self.pending += dt;
        let mut updates = 0;
        while self.pending >= UPDATE * 0.75 && updates < 4 {
            self.pending -= UPDATE;
            updates += 1;
        }
        self.pending = self.pending.clamp(-UPDATE / 2.0, UPDATE);
        if updates == 0 {
            return;
        }
        let [left, right] = &mut self.waves;
        let rate = match read(left, right) {
            Some(rate) if rate > 0.0 && rate.is_finite() => rate as f64,
            // No audio is what the page had before its AudioContext ran: no analysis at all, so
            // the bars hold still and the cadence doesn't move.
            _ => {
                self.pending = before;
                return;
            }
        };
        // The waves don't change between this tick's updates, so their silence is checked once.
        let silent = |wave: &[f32]| wave.iter().all(|&v| v == 0.0);
        let channels: &[usize] = if silent(&self.waves[1]) {
            &[0]
        } else if silent(&self.waves[0]) {
            &[1]
        } else {
            &[0, 1]
        };
        for j in 0..updates {
            // `this.sinceValues || TICK` in the JS: an exact 0 counts as unset, which changes when
            // the next analysis comes, so it is kept.
            let since = if self.since_values == 0.0 { TICK } else { self.since_values };
            self.since_values = since + UPDATE;
            if self.values.is_none() || self.since_values >= TICK * 0.75 {
                self.values = Some(self.levels((updates - 1 - j) as f64 * UPDATE, rate, channels));
                self.since_values = (self.since_values - TICK).max(0.0);
            }
            let values = self.values.unwrap();
            self.step(&values, UPDATE);
        }
    }

    // The bar levels (0..=15) from the audio `back` seconds before the end of the waves.
    fn levels(&mut self, back: f64, rate: f64, channels: &[usize]) -> [f64; BARS] {
        let ratio = rate / REF_RATE;
        let length = WAVE_LEN;
        let span = (2 * LINES - 1) as f64 * ratio;
        let end = span.max(length as f64 - 1.0 - back * rate);
        let start = end - span;
        for &c in channels {
            let wave = &self.waves[c];
            // Resampled to 44.1 kHz by linear interpolation: the bands are laid out for that rate.
            for n in 0..2 * LINES {
                let t = start + n as f64 * ratio;
                let i = t.floor();
                // `min` only matters above ~314 kHz, where the window no longer fits the waves.
                let a = wave[(i as usize).min(length - 1)] as f64;
                let b = wave[(i as usize + 1).min(length - 1)] as f64;
                self.work.block[n] = (a + (b - a) * (t - i)) * 32768.0 * self.tables.window[n];
            }
            self.mdct(c);
        }
        let mut columns = [0.0; COLUMNS + 1];
        let lines = &self.work.lines;
        for band in 0..COLUMNS {
            let from = BAND_EDGES[band];
            let to = BAND_EDGES[band + 1];
            let mut sum = 0.0;
            for k in from..to {
                sum += if channels.len() == 2 {
                    (lines[0][k].abs() + lines[1][k].abs()) * 0.5
                } else {
                    lines[channels[0]][k].abs()
                };
            }
            let weight = band as f64 * 12.0 / (to - from) as f64 + 1.0;
            columns[band] = js_round((sum * weight * 0.000075).clamp(0.0, 255.0));
        }
        // The last bar takes columns 72 to 75, and column 75 is always 0 (75 bands): as in the JS.
        let mut levels = [0.0; BARS];
        for (bar, level) in levels.iter_mut().enumerate() {
            let c = bar * 4;
            *level = ROWS.min(((columns[c] + columns[c + 1] + columns[c + 2] + columns[c + 3]) / 4.0).trunc());
        }
        levels
    }

    // MDCT of `work.block` (2 × LINES windowed samples) into `work.lines[out]`, through a
    // 288-point complex FFT.
    fn mdct(&mut self, out: usize) {
        let t = &*self.tables;
        let w = &mut *self.work;
        let x = &w.block;
        let u = &mut w.fold;
        for n in 0..HALF {
            u[n] = -x[3 * HALF - 1 - n] - x[3 * HALF + n];
            u[HALF + n] = x[n] - x[2 * HALF - 1 - n];
        }
        for m in 0..HALF {
            let ar = u[2 * m];
            let ai = u[LINES - 1 - 2 * m];
            w.re[m] = ar * t.pre_cos[m] + ai * t.pre_sin[m];
            w.im[m] = ai * t.pre_cos[m] - ar * t.pre_sin[m];
        }
        fft288(t, w);
        let scale = 2.0 / LINES as f64;
        let lines = &mut w.lines[out];
        for m in 0..HALF {
            let vr = w.out_re[m];
            let vi = w.out_im[m];
            lines[2 * m] = (vr * t.post_cos[m] + vi * t.post_sin[m]) * scale;
            lines[LINES - 1 - 2 * m] = -(vi * t.post_cos[m] - vr * t.post_sin[m]) * scale;
        }
    }

    fn step(&mut self, levels: &[f64; BARS], dt: f64) {
        let ticks = dt / TICK;
        let state = &mut self.state;
        for i in 0..BARS {
            let target = levels[i] * 16.0;
            state.bars[i] = if target >= state.bars[i] {
                target
            } else {
                target.max(state.bars[i] - BAR_FALLOFF * dt / BAR_TICK)
            };
            let shown = state.bars[i] * 16.0;
            if state.peaks[i] <= shown {
                state.peaks[i] = shown;
                state.tops[i] = shown;
                state.speed[i] = PEAK_SPEED;
            } else {
                state.peaks[i] = shown.max(state.peaks[i] - state.speed[i] * ticks);
                state.speed[i] *= GROWTH;
            }
        }
    }

    fn frame(&self) -> [u8; FRAME_LEN] {
        let mut frame = [0u8; FRAME_LEN];
        frame[0] = unit_byte(self.loader_level);
        if !self.off {
            for i in 0..BARS {
                let bars = &mut frame[1 + i * 4..];
                bars[0] = ROWS.min(js_round(self.state.bars[i] / 16.0)) as u8;
                let tops = self.state.tops[i];
                let peak_rows = js_round((tops - ((tops - self.state.peaks[i]) / 256.0).floor() * 256.0) / 256.0);
                if peak_rows >= 1.0 {
                    bars[1] = ROWS.min(peak_rows) as u8;
                }
            }
        }
        if self.loader_level > 0.0 {
            // A pulse that sweeps left to right every 1.1 s, starting and ending off the grid.
            let head = ((self.loader_time / 1.1) % 1.0) * (BARS + 6) as f64 - 3.0;
            for i in 0..BARS {
                let pulse = (1.0 - (i as f64 - head).abs() / 3.0).max(0.0);
                let loader = &mut frame[1 + i * 4 + 2..];
                loader[0] = (1.0 + js_round(pulse * 5.0)) as u8;
                loader[1] = js_round((0.3 + 0.7 * pulse) * 255.0) as u8;
            }
        }
        frame
    }

    /// For the reference test: the tables and the peak growth, which the JS computes with V8's
    /// sin, cos and pow.
    #[doc(hidden)]
    pub fn table_bits(&self) -> Vec<u64> {
        let t = &*self.tables;
        let tables: [&[f64]; 11] = [
            &t.window, &t.pre_cos, &t.pre_sin, &t.post_cos, &t.post_sin, &t.cos32, &t.sin32, &t.cos288, &t.sin288,
            &t.cos9, &t.sin9,
        ];
        let mut bits = Vec::new();
        for table in tables {
            bits.extend(table.iter().map(|v| v.to_bits()));
        }
        bits.push(GROWTH.to_bits());
        bits
    }
}

fn fft288(t: &Tables, w: &mut Work) {
    let Work { re, im, yr, yi, tr, ti, out_re, out_im, .. } = w;
    for n2 in 0..9 {
        for n1 in 0..32 {
            tr[t.rev32[n1]] = re[n1 * 9 + n2];
            ti[t.rev32[n1]] = im[n1 * 9 + n2];
        }
        let mut size = 2;
        while size <= 32 {
            let half = size >> 1;
            let step = 32 / size;
            for s in (0..32).step_by(size) {
                for k in 0..half {
                    let wr = t.cos32[k * step];
                    let wi = -t.sin32[k * step];
                    let a = s + k;
                    let b = a + half;
                    let xr = wr * tr[b] - wi * ti[b];
                    let xi = wr * ti[b] + wi * tr[b];
                    tr[b] = tr[a] - xr;
                    ti[b] = ti[a] - xi;
                    tr[a] += xr;
                    ti[a] += xi;
                }
            }
            size <<= 1;
        }
        for k1 in 0..32 {
            let c = t.cos288[k1 * 9 + n2];
            let s = t.sin288[k1 * 9 + n2];
            yr[k1 * 9 + n2] = tr[k1] * c + ti[k1] * s;
            yi[k1 * 9 + n2] = ti[k1] * c - tr[k1] * s;
        }
    }
    for k1 in 0..32 {
        for k2 in 0..9 {
            let mut sr = 0.0;
            let mut si = 0.0;
            for n2 in 0..9 {
                let i = (n2 * k2) % 9;
                let c = t.cos9[i];
                let s = t.sin9[i];
                sr += yr[k1 * 9 + n2] * c + yi[k1 * 9 + n2] * s;
                si += yi[k1 * 9 + n2] * c - yr[k1 * 9 + n2] * s;
            }
            out_re[k1 + 32 * k2] = sr;
            out_im[k1 + 32 * k2] = si;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn growth_is_the_power() {
        let pow = 1.6f64.powf(super::UPDATE / super::TICK);
        assert!((super::GROWTH.to_bits() as i64 - pow.to_bits() as i64).abs() <= 1);
    }

    #[test]
    fn js_round() {
        for (x, r) in [(0.5, 1.0), (-0.5, 0.0), (-2.5, -2.0), (2.4999999999999996, 2.0), (0.49999999999999994, 0.0), (7.5, 8.0)] {
            assert_eq!(super::js_round(x), r, "{x}");
        }
    }
}
