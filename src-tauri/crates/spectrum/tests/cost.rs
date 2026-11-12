// What a tick costs: it runs 60 times a second while anything plays. `ticks_never_allocate`
// always runs; the timing is for a release build:
//
//   cargo test -p qust-spectrum --release --test cost -- --ignored --nocapture

use qust_spectrum::{Spectrum, WAVE_LEN};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    // Only this thread's allocations count: the test harness allocates on its own threads.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(|c| c.get()) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// Two different channels (the costliest case: two MDCTs per analysis), 10 s at 48 kHz.
fn music() -> (Vec<f32>, Vec<f32>) {
    let rate = 48000.0;
    let mut seed = 1u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        seed as f32 / u32::MAX as f32 - 0.5
    };
    let (left, right) = (0..480000)
        .map(|n| {
            let t = n as f32 / rate;
            let tone = |f: f32| (t * f * std::f32::consts::TAU).sin();
            (
                0.3 * tone(110.0) + 0.2 * tone(1250.0) + 0.05 * tone(7000.0) + 0.05 * noise(),
                0.3 * tone(165.0) + 0.2 * tone(2500.0) + 0.05 * tone(9000.0) + 0.05 * noise(),
            )
        })
        .unzip();
    (left, right)
}

fn run(spectrum: &mut Spectrum, left: &[f32], right: &[f32], ticks: usize) -> u64 {
    let mut pos = WAVE_LEN;
    let mut checksum = 0u64;
    for _ in 0..ticks {
        let frame = spectrum.tick(1.0 / 60.0, &mut |l: &mut [f32], r: &mut [f32]| {
            l.copy_from_slice(&left[pos - WAVE_LEN..pos]);
            r.copy_from_slice(&right[pos - WAVE_LEN..pos]);
            Some(48000.0)
        });
        pos += 800;
        if pos > left.len() {
            pos = WAVE_LEN;
        }
        checksum += frame.iter().map(|&b| b as u64).sum::<u64>();
    }
    checksum
}

#[test]
fn ticks_never_allocate() {
    let (left, right) = music();
    let mut spectrum = Spectrum::new();
    spectrum.set_loading(true);
    COUNTING.with(|c| c.set(true));
    run(&mut spectrum, &left, &right, 300);
    spectrum.set_loading(false);
    spectrum.reset();
    run(&mut spectrum, &left, &right, 300);
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0, "tick allocated");
}

#[test]
#[ignore]
fn tick_time() {
    let (left, right) = music();
    let mut spectrum = Spectrum::new();
    run(&mut spectrum, &left, &right, 600);
    let ticks = 6000;
    let start = Instant::now();
    let checksum = run(&mut spectrum, &left, &right, ticks);
    let each = start.elapsed().as_secs_f64() / ticks as f64 * 1e6;
    println!("{each:.1} µs per tick at 60 Hz, reading included (checksum {checksum})");
}
