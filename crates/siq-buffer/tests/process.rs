//! The process's counters on the real system: its memory, its processors' time, and the bytes it
//! wrote, each growing with the work done.

use std::hint::black_box;
use std::time::{Duration, Instant};

use siq_buffer::{process_memory, process_usage};

// REQ-NFR-002
#[test]
fn the_process_s_time_and_bytes_written_grow_with_its_work() {
    let before = process_usage().unwrap();
    // A quarter second of arithmetic on this thread.
    let began = Instant::now();
    let mut sum = 0u64;
    while began.elapsed() < Duration::from_millis(250) {
        for k in 0..10_000u64 {
            sum = black_box(sum.wrapping_add(k * k));
        }
    }
    // And 8 MB written.
    let path = std::env::temp_dir().join(format!("siq-buffer-usage-{}", std::process::id()));
    std::fs::write(&path, vec![7u8; 8 << 20]).unwrap();
    std::fs::remove_file(&path).unwrap();
    let after = process_usage().unwrap();
    let cpu = after.cpu - before.cpu;
    assert!(
        cpu >= Duration::from_millis(150),
        "{cpu:?} of the processors"
    );
    let written = after.written_bytes - before.written_bytes;
    assert!(written >= 8 << 20, "{written} bytes written");
    assert!(after.read_bytes >= before.read_bytes);

    let memory = process_memory().unwrap();
    assert!(memory.peak_working_set >= memory.working_set && memory.working_set > 0);
}
