// sin and cos as V8 computes Math.sin and Math.cos (src/base/ieee754.cc, from fdlibm 5.3), for the
// MDCT tables. The platform's libm is just as accurate but rounds about 1 value in 30 the other
// way, and the tables should be the ones the page used. Only arguments below 2^19·π/2 are needed
// (the tables stay under 56), so the large-argument reduction is left to std.

// The constants are written as in fdlibm, digits and all, so they can be checked against it.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

const HALF: f64 = 0.5;

fn high_word(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn kernel_sin(x: f64, y: f64, iy: bool) -> f64 {
    const S1: f64 = -1.66666666666666324348e-01;
    const S2: f64 = 8.33333333332248946124e-03;
    const S3: f64 = -1.98412698298579493134e-04;
    const S4: f64 = 2.75573137070700676789e-06;
    const S5: f64 = -2.50507602534068634195e-08;
    const S6: f64 = 1.58969099521155010221e-10;
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix < 0x3E400000 && x as i32 == 0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if !iy {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (HALF * y - v * r) - y) - v * S1)
    }
}

fn kernel_cos(x: f64, y: f64) -> f64 {
    const C1: f64 = 4.16666666666666019037e-02;
    const C2: f64 = -1.38888888888741095749e-03;
    const C3: f64 = 2.48015872894767294178e-05;
    const C4: f64 = -2.75573143513906633035e-07;
    const C5: f64 = 2.08757232129817482790e-09;
    const C6: f64 = -1.13596475577881948265e-11;
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix < 0x3E400000 && x as i32 == 0 {
        return 1.0;
    }
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    if ix < 0x3FD33333 {
        1.0 - (0.5 * z - (z * r - x * y))
    } else {
        let qx = if ix > 0x3FE90000 { 0.28125 } else { f64::from_bits(((ix - 0x00200000) as u64) << 32) };
        let iz = 0.5 * z - qx;
        let a = 1.0 - qx;
        a - (iz - (z * r - x * y))
    }
}

// x = n·π/2 + y.0 + y.1, for |x| above π/4 and up to 2^19·π/2.
fn rem_pio2(x: f64) -> (i32, f64, f64) {
    const NPIO2_HW: [i32; 32] = [
        0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB, 0x402921FB,
        0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB,
        0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x4041475C, 0x4042106C, 0x4042D97C,
        0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
    ];
    const INVPIO2: f64 = 6.36619772367581382433e-01;
    const PIO2_1: f64 = 1.57079632673412561417e+00;
    const PIO2_1T: f64 = 6.07710050650619224932e-11;
    const PIO2_2: f64 = 6.07710050630396597660e-11;
    const PIO2_2T: f64 = 2.02226624879595063154e-21;
    const PIO2_3: f64 = 2.02226624871116645580e-21;
    const PIO2_3T: f64 = 8.47842766036889956997e-32;
    let hx = high_word(x);
    let ix = hx & 0x7FFFFFFF;
    if ix < 0x4002D97C {
        // |x| < 3π/4: n = ±1.
        return if hx > 0 {
            let mut z = x - PIO2_1;
            if ix != 0x3FF921FB {
                let y0 = z - PIO2_1T;
                (1, y0, (z - y0) - PIO2_1T)
            } else {
                z -= PIO2_2;
                let y0 = z - PIO2_2T;
                (1, y0, (z - y0) - PIO2_2T)
            }
        } else {
            let mut z = x + PIO2_1;
            if ix != 0x3FF921FB {
                let y0 = z + PIO2_1T;
                (-1, y0, (z - y0) + PIO2_1T)
            } else {
                z += PIO2_2;
                let y0 = z + PIO2_2T;
                (-1, y0, (z - y0) + PIO2_2T)
            }
        };
    }
    let t = x.abs();
    let n = (t * INVPIO2 + HALF) as i32;
    let f = n as f64;
    let mut r = t - f * PIO2_1;
    let mut w = f * PIO2_1T;
    let mut y0;
    if n < 32 && ix != NPIO2_HW[n as usize - 1] {
        y0 = r - w;
    } else {
        let j = ix >> 20;
        y0 = r - w;
        let i = j - ((high_word(y0) >> 20) & 0x7FF);
        if i > 16 {
            let t = r;
            w = f * PIO2_2;
            r = t - w;
            w = f * PIO2_2T - ((t - r) - w);
            y0 = r - w;
            let i = j - ((high_word(y0) >> 20) & 0x7FF);
            if i > 49 {
                let t = r;
                w = f * PIO2_3;
                r = t - w;
                w = f * PIO2_3T - ((t - r) - w);
                y0 = r - w;
            }
        }
    }
    let y1 = (r - y0) - w;
    if hx < 0 {
        (-n, -y0, -y1)
    } else {
        (n, y0, y1)
    }
}

fn in_range(ix: i32) -> bool {
    ix <= 0x413921FB
}

pub fn sin(x: f64) -> f64 {
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix <= 0x3FE921FB {
        return kernel_sin(x, 0.0, false);
    }
    if !in_range(ix) {
        return x.sin();
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_sin(y0, y1, true),
        1 => kernel_cos(y0, y1),
        2 => -kernel_sin(y0, y1, true),
        _ => -kernel_cos(y0, y1),
    }
}

pub fn cos(x: f64) -> f64 {
    let ix = high_word(x) & 0x7FFFFFFF;
    if ix <= 0x3FE921FB {
        return kernel_cos(x, 0.0);
    }
    if !in_range(ix) {
        return x.cos();
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_cos(y0, y1),
        1 => -kernel_sin(y0, y1, true),
        2 => -kernel_cos(y0, y1),
        _ => kernel_sin(y0, y1, true),
    }
}
