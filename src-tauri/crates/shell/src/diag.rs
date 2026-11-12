// Process counters for the measurements (examples/idle.rs) and the tests: CPU time, memory, and
// the context switches of every thread, which count how often the process wakes up.

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub cpu_ms: f64,
    pub private_bytes: usize,
    pub working_set: usize,
    pub threads: usize,
    pub context_switches: u64,
    // (thread id, context switches, CPU ms)
    pub per_thread: Vec<(usize, u64, f64)>,
}

impl Sample {
    // Wakeups and CPU ms of every thread but `except` (the sampler itself) since `before`.
    pub fn activity_since(&self, before: &Sample, except: usize) -> (u64, f64) {
        let mut switches = 0;
        let mut cpu = 0.0;
        for (id, s, c) in &self.per_thread {
            if *id == except {
                continue;
            }
            let (old_s, old_c) = before.per_thread.iter().find(|(old, _, _)| old == id).map(|(_, s, c)| (*s, *c)).unwrap_or((0, 0.0));
            switches += s.saturating_sub(old_s);
            cpu += (c - old_c).max(0.0);
        }
        (switches, cpu)
    }

    // The threads that woke most between `before` and this sample: (thread id, wakeups, CPU ms).
    pub fn busiest(&self, before: &Sample, count: usize) -> Vec<(usize, u64, f64)> {
        let mut deltas: Vec<(usize, u64, f64)> = self
            .per_thread
            .iter()
            .map(|(id, switches, cpu)| {
                let (old_switches, old_cpu) = before.per_thread.iter().find(|(old, _, _)| old == id).map(|(_, s, c)| (*s, *c)).unwrap_or((0, 0.0));
                (*id, switches.saturating_sub(old_switches), cpu - old_cpu)
            })
            .collect();
        deltas.sort_by(|a, b| b.1.cmp(&a.1));
        deltas.truncate(count);
        deltas
    }
}

// This thread's id.
#[cfg(windows)]
pub fn current_thread() -> usize {
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() as usize }
}

#[cfg(not(windows))]
pub fn current_thread() -> usize {
    0
}

#[cfg(windows)]
pub fn sample() -> Sample {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut sample = Sample::default();
    unsafe {
        let process = GetCurrentProcess();
        let mut memory = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let _ = GetProcessMemoryInfo(process, &mut memory as *mut _ as *mut PROCESS_MEMORY_COUNTERS, memory.cb);
        sample.private_bytes = memory.PrivateUsage;
        sample.working_set = memory.WorkingSetSize;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let ms = |time: FILETIME| ((time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64) as f64 / 10_000.0;
        sample.cpu_ms = ms(kernel) + ms(user);
    }
    for (id, switches, cpu) in threads() {
        sample.threads += 1;
        sample.context_switches += switches;
        sample.per_thread.push((id, switches, cpu));
    }
    sample
}

// The threads of this process from NtQuerySystemInformation(SystemProcessInformation): (id,
// context switches, CPU ms). The structures aren't in the windows crate; the offsets are those of
// SYSTEM_PROCESS_INFORMATION and SYSTEM_THREAD_INFORMATION on 64-bit Windows.
#[cfg(all(windows, target_pointer_width = "64"))]
fn threads() -> Vec<(usize, u64, f64)> {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQuerySystemInformation(class: u32, buffer: *mut u8, length: u32, returned: *mut u32) -> i32;
    }
    const SYSTEM_PROCESS_INFORMATION: u32 = 5;
    const PROCESS_HEADER: usize = 0x100;
    const THREAD_SIZE: usize = 0x50;

    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let mut returned = 0u32;
        let status = unsafe { NtQuerySystemInformation(SYSTEM_PROCESS_INFORMATION, buffer.as_mut_ptr(), buffer.len() as u32, &mut returned) };
        if status == 0 {
            break;
        }
        // STATUS_INFO_LENGTH_MISMATCH: grow and retry.
        if status as u32 == 0xC000_0004 && buffer.len() < (64 << 20) {
            buffer = vec![0u8; buffer.len().max(returned as usize) * 2];
            continue;
        }
        return Vec::new();
    }
    let read_u32 = |at: usize| u32::from_ne_bytes(buffer[at..at + 4].try_into().unwrap_or([0; 4]));
    let read_u64 = |at: usize| u64::from_ne_bytes(buffer[at..at + 8].try_into().unwrap_or([0; 8]));
    let pid = std::process::id() as u64;
    let mut offset = 0usize;
    loop {
        if offset + PROCESS_HEADER > buffer.len() {
            return Vec::new();
        }
        let next = read_u32(offset) as usize;
        let count = read_u32(offset + 4) as usize;
        if read_u64(offset + 0x50) == pid {
            let mut out = Vec::with_capacity(count);
            for index in 0..count {
                let thread = offset + PROCESS_HEADER + index * THREAD_SIZE;
                if thread + THREAD_SIZE > buffer.len() {
                    break;
                }
                let cpu = (read_u64(thread) + read_u64(thread + 8)) as f64 / 10_000.0;
                let id = read_u64(thread + 0x30) as usize;
                let switches = read_u32(thread + 0x40) as u64;
                out.push((id, switches, cpu));
            }
            return out;
        }
        if next == 0 {
            return Vec::new();
        }
        offset += next;
    }
}

#[cfg(all(windows, not(target_pointer_width = "64")))]
fn threads() -> Vec<(usize, u64, f64)> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn sample() -> Sample {
    Sample::default()
}

// A hidden window and GetMessage until WM_QUIT: the cheapest a process with a tray icon can be.
#[cfg(windows)]
pub fn plain_message_loop() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};
    let _window = crate::win::HiddenWindow::new("QustIdleProbe", None);
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
pub fn plain_message_loop() {
    loop {
        std::thread::park();
    }
}
