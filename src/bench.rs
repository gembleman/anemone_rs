//! paint() 마이크로벤치마크
//!
//! `ANEMONE_BENCH_PAINT=<N>` 환경변수를 설정하면 앱 시작 시 paint() 본체를
//! N 회 반복 측정하여 평균/min/p50/max 를 tracing 으로 출력한다. D2D 합성
//! 경로 재작성 (HwndRT / DComp) 결정을 위한 baseline 측정용.
//!
//! `next_steps.md` 1 항 권장 절차에 대응.

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// QueryPerformanceCounter 기반 고해상도 타이머
pub struct QpcTimer {
    frequency: i64,
}

impl QpcTimer {
    pub fn new() -> Self {
        let mut freq: i64 = 0;
        // SAFETY: QueryPerformanceFrequency 는 항상 성공하며 freq 에 cpu tick 빈도를 쓴다.
        unsafe {
            let _ = QueryPerformanceFrequency(&mut freq);
        }
        Self { frequency: freq.max(1) }
    }

    #[inline]
    pub fn now(&self) -> i64 {
        let mut t: i64 = 0;
        // SAFETY: out 포인터는 로컬 스택 변수이며 함수는 항상 성공한다.
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        t
    }

    /// tick 차이를 마이크로초로 변환
    #[inline]
    pub fn to_micros(&self, ticks: i64) -> f64 {
        (ticks as f64) * 1_000_000.0 / (self.frequency as f64)
    }
}

/// 측정 샘플을 모아 통계로 요약한다.
pub struct BenchAccumulator {
    samples: Vec<i64>,
    timer: QpcTimer,
}

impl BenchAccumulator {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            samples: Vec::with_capacity(n),
            timer: QpcTimer::new(),
        }
    }

    #[inline]
    pub fn timer(&self) -> &QpcTimer {
        &self.timer
    }

    #[inline]
    pub fn push(&mut self, ticks: i64) {
        self.samples.push(ticks);
    }

    /// 통계 출력 — tracing 으로 로그하고, GUI 서브시스템이라 콘솔이 없어도
    /// 결과를 잃지 않도록 실행파일 옆 `bench_paint.log` 에 append 한다.
    pub fn report(&mut self, label: &str) {
        if self.samples.is_empty() {
            tracing::info!("[bench {label}] (샘플 없음)");
            return;
        }

        self.samples.sort_unstable();
        let n = self.samples.len();
        let sum: i64 = self.samples.iter().sum();
        let min = self.samples[0];
        let max = self.samples[n - 1];
        let p50 = self.samples[n / 2];
        let p99 = self.samples[(n * 99 / 100).min(n - 1)];
        let avg = sum / n as i64;

        let line = format!(
            "[bench {label}] n={n} avg={:.1}us p50={:.1}us p99={:.1}us min={:.1}us max={:.1}us",
            self.timer.to_micros(avg),
            self.timer.to_micros(p50),
            self.timer.to_micros(p99),
            self.timer.to_micros(min),
            self.timer.to_micros(max),
        );

        tracing::info!("{line}");

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

/// 실행파일 옆 `bench_paint.log` 경로
fn bench_log_path() -> Option<std::path::PathBuf> {
    let mut p = std::env::current_exe().ok()?;
    p.pop();
    p.push("bench_paint.log");
    Some(p)
}

/// `ANEMONE_BENCH_PAINT` 환경변수에서 반복 횟수를 읽는다. 미설정/0 이면 None.
pub fn paint_bench_iters() -> Option<usize> {
    let raw = std::env::var("ANEMONE_BENCH_PAINT").ok()?;
    let n: usize = raw.trim().parse().ok()?;
    if n == 0 { None } else { Some(n) }
}

/// paint() 내부의 phase 별 비용을 누적하는 thread-local 슬롯.
///
/// `BenchPhaseRecorder::activate()` 로 켜진 동안에만 paint() 코드의 hook
/// 지점 (`record_phase!`) 이 실제 측정을 수행하고, 비활성 시에는 None 체크
/// 1 회 + early return 으로 overhead 가 거의 0 이다. 정상 운용 paint 경로에
/// 이 인프라가 끼어들지 않게 하는 게 핵심.
///
/// **phase 구성 (정의 순서)**:
/// - `lazy_init` — CompositionRenderer 첫 부착 (정상 케이스는 0)
/// - `swap_chain_wait` — frame latency wait (waitable swap chain)
/// - `setup` — config 읽기 + render style 준비
/// - `begin_clear` — BeginDraw + 배경 클리어
/// - `border` — 테두리 그리기 (border_visible 시)
/// - `text` — 텍스트 그리기 (DirectWrite + outline geometry)
/// - `end_draw` — EndDraw (GPU 명령 큐 flush + BeginDraw 짝 닫기)
/// - `present` — swap chain Present
/// - `hit_region` — hit-test 사각형 갱신 (`background_visible=false` 시)
pub struct PhaseRecord {
    timer: QpcTimer,
    pub lazy_init: i64,
    pub swap_chain_wait: i64,
    pub setup: i64,
    pub begin_clear: i64,
    pub border: i64,
    pub text: i64,
    pub end_draw: i64,
    pub present: i64,
    pub hit_region: i64,
}

impl PhaseRecord {
    pub fn new() -> Self {
        Self {
            timer: QpcTimer::new(),
            lazy_init: 0,
            swap_chain_wait: 0,
            setup: 0,
            begin_clear: 0,
            border: 0,
            text: 0,
            end_draw: 0,
            present: 0,
            hit_region: 0,
        }
    }

    /// 직전 `now()` 시점으로부터 경과 tick 을 phase 필드에 누적.
    #[inline]
    pub fn add(&mut self, field: PhaseField, since: i64) -> i64 {
        let t = self.timer.now();
        let delta = t - since;
        match field {
            PhaseField::LazyInit => self.lazy_init += delta,
            PhaseField::SwapChainWait => self.swap_chain_wait += delta,
            PhaseField::Setup => self.setup += delta,
            PhaseField::BeginClear => self.begin_clear += delta,
            PhaseField::Border => self.border += delta,
            PhaseField::Text => self.text += delta,
            PhaseField::EndDraw => self.end_draw += delta,
            PhaseField::Present => self.present += delta,
            PhaseField::HitRegion => self.hit_region += delta,
        }
        t
    }

    #[inline]
    pub fn timer(&self) -> &QpcTimer {
        &self.timer
    }
}

#[derive(Copy, Clone)]
pub enum PhaseField {
    LazyInit,
    SwapChainWait,
    Setup,
    BeginClear,
    Border,
    Text,
    EndDraw,
    Present,
    HitRegion,
}

thread_local! {
    /// 활성화되면 paint() 의 hook 들이 여기 누적한다. 비활성 시 None.
    static PHASE_RECORDER: std::cell::RefCell<Option<PhaseRecord>> =
        const { std::cell::RefCell::new(None) };
}

/// phase 측정을 시작 (recorder 새로 만들어 슬롯에 보관).
pub fn phase_begin() {
    PHASE_RECORDER.with(|cell| {
        *cell.borrow_mut() = Some(PhaseRecord::new());
    });
}

/// phase 측정을 종료하고 누적된 레코드를 꺼낸다.
pub fn phase_end() -> Option<PhaseRecord> {
    PHASE_RECORDER.with(|cell| cell.borrow_mut().take())
}

/// hook 지점에서 호출. recorder 가 활성화된 동안만 측정을 수행한다.
///
/// 사용 패턴:
/// ```ignore
/// let t0 = phase_now(); // 시점 마킹
/// /* phase 작업 */
/// let t0 = phase_record(PhaseField::Setup, t0); // 누적 + 다음 phase 시작점 반환
/// /* 다음 phase */
/// let _ = phase_record(PhaseField::BeginClear, t0);
/// ```
#[inline]
pub fn phase_now() -> i64 {
    PHASE_RECORDER.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(rec) => rec.timer().now(),
            None => 0,
        }
    })
}

#[inline]
pub fn phase_record(field: PhaseField, since: i64) -> i64 {
    PHASE_RECORDER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match borrow.as_mut() {
            Some(rec) => rec.add(field, since),
            None => 0,
        }
    })
}

/// detailed phase 벤치 누적기. 한 번의 paint 마다 각 phase 시간을 별도 vec 에
/// 모아 paint 전체 분포와 별개로 phase 별 통계를 낼 수 있게 한다.
pub struct PhasedBenchAccumulator {
    pub lazy_init: BenchAccumulator,
    pub swap_chain_wait: BenchAccumulator,
    pub setup: BenchAccumulator,
    pub begin_clear: BenchAccumulator,
    pub border: BenchAccumulator,
    pub text: BenchAccumulator,
    pub end_draw: BenchAccumulator,
    pub present: BenchAccumulator,
    pub hit_region: BenchAccumulator,
    pub total: BenchAccumulator,
}

impl PhasedBenchAccumulator {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            lazy_init: BenchAccumulator::with_capacity(n),
            swap_chain_wait: BenchAccumulator::with_capacity(n),
            setup: BenchAccumulator::with_capacity(n),
            begin_clear: BenchAccumulator::with_capacity(n),
            border: BenchAccumulator::with_capacity(n),
            text: BenchAccumulator::with_capacity(n),
            end_draw: BenchAccumulator::with_capacity(n),
            present: BenchAccumulator::with_capacity(n),
            hit_region: BenchAccumulator::with_capacity(n),
            total: BenchAccumulator::with_capacity(n),
        }
    }

    pub fn push(&mut self, rec: &PhaseRecord, total_ticks: i64) {
        self.lazy_init.push(rec.lazy_init);
        self.swap_chain_wait.push(rec.swap_chain_wait);
        self.setup.push(rec.setup);
        self.begin_clear.push(rec.begin_clear);
        self.border.push(rec.border);
        self.text.push(rec.text);
        self.end_draw.push(rec.end_draw);
        self.present.push(rec.present);
        self.hit_region.push(rec.hit_region);
        self.total.push(total_ticks);
    }

    pub fn report(&mut self) {
        self.total.report("paint_detailed_total");
        self.lazy_init.report("paint_detailed_lazy_init");
        self.swap_chain_wait.report("paint_detailed_swap_chain_wait");
        self.setup.report("paint_detailed_setup");
        self.begin_clear.report("paint_detailed_begin_clear");
        self.border.report("paint_detailed_border");
        self.text.report("paint_detailed_text");
        self.end_draw.report("paint_detailed_end_draw");
        self.present.report("paint_detailed_present");
        self.hit_region.report("paint_detailed_hit_region");
    }
}

/// `ANEMONE_BENCH_PAINT_DETAILED` 환경변수에서 반복 횟수를 읽는다. 미설정/0 이면 None.
///
/// `ANEMONE_BENCH_PAINT` 와 동시에 설정되면 detailed 측정만 수행한다 (이쪽이
/// 더 많은 정보를 제공하므로). detailed 측정은 paint() 내부 hook 의 RefCell
/// borrow + thread_local 접근 overhead 가 있어 paint 전체 시간 자체는 일반
/// bench 보다 약간 더 느리게 측정될 수 있다 — phase 별 비율 비교용으로만
/// 해석해야 한다.
pub fn paint_bench_detailed_iters() -> Option<usize> {
    let raw = std::env::var("ANEMONE_BENCH_PAINT_DETAILED").ok()?;
    let n: usize = raw.trim().parse().ok()?;
    if n == 0 { None } else { Some(n) }
}

/// 벤치 측정 시 outline/shadow 를 강제 비활성화한다. `text`/`end_draw`
/// 비용의 출처가 outline/shadow geometry 인지 텍스트 본문 (`DrawTextLayout`)
/// 인지 분리 측정하기 위한 토글.
///
/// 효과:
/// - `style.outline1_size = 0`
/// - `style.outline2_size = 0`
/// - `style.shadow_enabled = false`
///
/// 정상 동작 paint 에는 영향 없음 — bench 함수 내부에서만 임시 override.
pub fn bench_disable_outline() -> bool {
    matches!(
        std::env::var("ANEMONE_BENCH_PAINT_NO_OUTLINE")
            .ok()
            .as_deref(),
        Some("1") | Some("true")
    )
}

/// 벤치 측정 시 매 iteration 마다 텍스트를 살짝 바꿔 outline 비트맵 /
/// layout / outline geometry 캐시 (항목 8, 10) 를 강제 miss 시킨다. 항목
/// 10 알려진 한계 — "텍스트가 ms 단위로 폭주 변경되면 매 paint 가 캐시
/// miss 가 되어 baseline 보다 느려질 위험" — 의 실측 검증용.
///
/// 효과 (`run_paint_bench_detailed` 측정 루프 안에서):
/// - 매 iteration 진입 직전 `current_text` 끝에 카운터 (`#0`, `#1`, …)
///   를 붙여 캐시 키를 매번 다르게 만든다.
/// - 측정 종료 후 원래 텍스트로 복구한다.
///
/// 정상 paint 경로는 무변경 — bench 함수 안에서만 임시 mutate.
pub fn bench_force_cache_miss() -> bool {
    matches!(
        std::env::var("ANEMONE_BENCH_PAINT_CACHE_MISS")
            .ok()
            .as_deref(),
        Some("1") | Some("true")
    )
}
