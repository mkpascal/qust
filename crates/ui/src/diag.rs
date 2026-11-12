//! Process counters for measurements (CPU time, memory). Windows only; elsewhere zeros.

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    /// User + kernel CPU time of the whole process, ms.
    pub cpu_ms: f64,
    /// Private bytes.
    pub private: usize,
    pub working_set: usize,
}

#[cfg(windows)]
pub fn counters() -> Counters {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    unsafe {
        let process = GetCurrentProcess();
        let mut memory = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let _ = GetProcessMemoryInfo(process, &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS, memory.cb);
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let ms = |t: FILETIME| (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 10_000.0;
        Counters { cpu_ms: ms(kernel) + ms(user), private: memory.PrivateUsage, working_set: memory.WorkingSetSize }
    }
}

#[cfg(not(windows))]
pub fn counters() -> Counters {
    Counters::default()
}

/// Writes a 16-bit stereo 44.1 kHz WAV of `seconds` of a few tones (for test runs on the null
/// output: something the analyzer has bars for).
pub fn write_tone_wav(path: &std::path::Path, seconds: f64) -> std::io::Result<()> {
    let rate = 44_100u32;
    let frames = (seconds * rate as f64) as u32;
    let mut data = Vec::with_capacity(frames as usize * 4 + 44);
    let header = |data: &mut Vec<u8>| {
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&rate.to_le_bytes());
        data.extend_from_slice(&(rate * 4).to_le_bytes());
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        data.extend_from_slice(b"data");
        data.extend_from_slice(&(frames * 4).to_le_bytes());
    };
    header(&mut data);
    for i in 0..frames {
        let t = i as f64 / rate as f64;
        // Low, mid and high tones that come and go, so bars move.
        let wobble = 0.5 + 0.5 * (t * 1.7).sin();
        let s = 0.3 * (2.0 * std::f64::consts::PI * 110.0 * t).sin() + 0.2 * wobble * (2.0 * std::f64::consts::PI * 1200.0 * t).sin() + 0.1 * (2.0 * std::f64::consts::PI * 6000.0 * t).sin() * (1.0 - wobble);
        let sample = ((s * 0.8).clamp(-1.0, 1.0) * i16::MAX as f64) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, data)
}
