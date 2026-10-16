//! Local, opt-in allocation comparison. Never emits media, URLs or file names.
//! This measures Rust requested heap bytes, not PSS or platform codec memory.
use atv_player::playback::MediaProcessor;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hash::{Hash, Hasher},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

struct Meter;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
fn add(bytes: usize) {
    let current = CURRENT.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(current, Relaxed);
    ALLOCATIONS.fetch_add(1, Relaxed);
}
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            CURRENT.fetch_sub(layout.size(), Relaxed);
            add(size);
        }
        result
    }
}
#[global_allocator]
static HEAP: Meter = Meter;

fn main() -> Result<(), &'static str> {
    let mut args = std::env::args().skip(1);
    let batch_packets: usize = args
        .next()
        .ok_or("expected batch size (0 = eager), then TS files")?
        .parse()
        .map_err(|_| "invalid batch size")?;
    let paths: Vec<_> = args.collect();
    if paths.is_empty() {
        return Err("expected local authorized TS files");
    }
    let inputs: Vec<Vec<u8>> = paths
        .iter()
        .map(|path| std::fs::read(path).map_err(|_| "local media unavailable"))
        .collect::<Result<_, _>>()?;
    if inputs
        .iter()
        .any(|data| data.is_empty() || !data.len().is_multiple_of(188) || data[0] != 0x47)
    {
        return Err("profile requires packet-aligned TS inputs");
    }
    let chunk_bytes = if batch_packets == 0 {
        usize::MAX
    } else {
        batch_packets
            .checked_mul(188)
            .ok_or("batch size overflow")?
    };
    let baseline = CURRENT.load(Relaxed);
    let allocations_before = ALLOCATIONS.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    let started = std::time::Instant::now();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut units = 0u64;
    let mut elementary_bytes = 0u64;
    for _ in 0..20 {
        let mut processor = MediaProcessor::new(String::new());
        for data in &inputs {
            for chunk in data.chunks(chunk_bytes) {
                for unit in processor
                    .fragment(chunk)
                    .map_err(|_| "media processing failed")?
                {
                    (unit.codec as u8).hash(&mut hash);
                    unit.pts_us.hash(&mut hash);
                    unit.data.hash(&mut hash);
                    units += 1;
                    elementary_bytes += unit.data.len() as u64;
                }
            }
        }
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let peak_extra = PEAK.load(Relaxed).saturating_sub(baseline);
    let residual = CURRENT.load(Relaxed).saturating_sub(baseline);
    let allocations = ALLOCATIONS.load(Relaxed) - allocations_before;
    println!(
        "{}",
        serde_json::json!({
            "evidence": "local-media-Rust-requested-heap-only", "batchPackets": batch_packets,
            "replays":20, "inputBytes":inputs.iter().map(Vec::len).sum::<usize>(),
            "peakAdditionalRequestedHeapBytes":peak_extra, "residualRequestedHeapBytes":residual,
            "allocationsAndReallocations":allocations, "elapsedMs":elapsed_ms,
            "accessUnits":units, "elementaryBytes":elementary_bytes, "outputHash":hash.finish(),
            "limitations":"Input file allocations excluded equally; no network, decoder, Surface, allocator metadata or RSS/PSS. Hash verifies local equivalence, not independent decode correctness."
        })
    );
    Ok(())
}
