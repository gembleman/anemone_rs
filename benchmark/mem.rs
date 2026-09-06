//! 벤치마크용 할당 계측기 — 프로세스 전역 할당을 누적/현재/피크로 센다.
//!
//! `CountingAllocator`는 어떤 크레이트에서도 컴파일되지만, **`#[global_allocator]`
//! 선언은 바이너리 크레이트에서 해야 한다** (lib에는 지정 금지). 벤치 바이너리
//! 각각에서 아래처럼 선언한다:
//!
//! ```rust
//! #[global_allocator]
//! static ALLOCATOR: bench_mem::CountingAllocator = bench_mem::CountingAllocator;
//! ```
//!
//! 카운터는 프로세스 전역 Atomic이므로 "작업 전 스냅샷 → 작업 → 작업 후 스냅샷"의
//! 델타로 읽는다. 벤치 피처 없이 빌드되면 어느 바이너리에도 적용되지 않는다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

static ALLOC_TOTAL: AtomicU64 = AtomicU64::new(0);
static ALLOC_CURRENT: AtomicU64 = AtomicU64::new(0);
static ALLOC_PEAK: AtomicU64 = AtomicU64::new(0);

/// 벤치 바이너리의 `#[global_allocator]`로 지정하는 계측 할당자.
pub struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 위임 호출 — System 할당자는 유효 포인터를 반환하거나 null이다.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            track_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_CURRENT.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: GlobalAlloc 계약에 따라 유효한 할당/해제 쌍이다.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: GlobalAlloc 계약에 따라 유효한 할당/재할당 쌍이다.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let old = layout.size() as i64;
            let new = new_size as i64;
            match new - old {
                0 => {}
                delta if delta > 0 => track_alloc(delta as usize),
                delta => {
                    ALLOC_CURRENT.fetch_sub((-delta) as u64, Ordering::Relaxed);
                }
            }
        }
        new_ptr
    }
}

fn track_alloc(size: usize) {
    ALLOC_TOTAL.fetch_add(size as u64, Ordering::Relaxed);
    let current = ALLOC_CURRENT.fetch_add(size as u64, Ordering::Relaxed) + size as u64;
    ALLOC_PEAK.fetch_max(current, Ordering::Relaxed);
}

/// 특정 시점의 할당 카운터 스냅샷.
#[derive(Clone, Copy, Default)]
pub struct MemSnapshot {
    /// 프로세스 시작 이후 누적 할당 바이트.
    pub total_bytes: u64,
    /// 현재 살아있는 할당 바이트 (해제 안 된 것).
    pub live_bytes: u64,
    /// 지금까지의 최대 동시 상주 바이트.
    pub peak_bytes: u64,
}

impl MemSnapshot {
    pub fn now() -> Self {
        Self {
            total_bytes: ALLOC_TOTAL.load(Ordering::Relaxed),
            live_bytes: ALLOC_CURRENT.load(Ordering::Relaxed),
            peak_bytes: ALLOC_PEAK.load(Ordering::Relaxed),
        }
    }

    /// 작업 구간의 델타 — `end - start`.
    ///
    /// `total_delta`는 작업이 만든 누적 할당(churn), `peak`는 작업 중 최대
    /// 상주량이다. `live_delta`는 작업 후에도 남아있는 잔여 상주(캐시 등)다.
    pub fn delta(&self, start: MemSnapshot) -> MemDelta {
        MemDelta {
            total_delta: self.total_bytes - start.total_bytes,
            live_delta: self.live_bytes.saturating_sub(start.live_bytes),
            peak: self.peak_bytes.max(start.peak_bytes),
        }
    }
}

pub struct MemDelta {
    pub total_delta: u64,
    pub live_delta: u64,
    pub peak: u64,
}

impl MemDelta {
    /// 작업 단위가 알려진 경우 단위당 churn을 함께 보고한다.
    ///
    /// stderr와 실행 파일 옆 `bench_paint.log`(paint 벤치와 같은 경로)에 쓴다 —
    /// release 바이너리는 `windows_subsystem`으로 콘솔이 없어 파일이 필요하다.
    pub fn report_per(&self, label: &str, units: usize) {
        let per = if units == 0 {
            0.0
        } else {
            self.total_delta as f64 / units as f64
        };
        let line = format!(
            "[mem {label}] churn={:.1}MB ({:.0}B/unit) live_delta={:.1}MB peak={:.1}MB",
            self.total_delta as f64 / 1_048_576.0,
            per,
            self.live_delta as f64 / 1_048_576.0,
            self.peak as f64 / 1_048_576.0,
        );
        eprintln!("{line}");
        if let Some(path) = bench_log_path() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

/// 실행파일 옆 `bench_paint.log` 경로 — `benchmark/bench.rs`와 같은 관례.
fn bench_log_path() -> Option<std::path::PathBuf> {
    let mut p = std::env::current_exe().ok()?;
    p.pop();
    p.push("bench_paint.log");
    Some(p)
}
