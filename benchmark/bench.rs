//! paint() 마이크로벤치마크
//!
//! `ANEMONE_BENCH_PAINT=<N>` 환경변수를 설정하면 앱 시작 시 paint() 본체를
//! N 회 반복 측정하여 평균/min/p50/max 를 tracing 으로 출력한다. D2D 합성
//! 경로 재작성 (HwndRT / DComp) 결정을 위한 baseline 측정용.
//!
//! `next_steps.md` 1 항 권장 절차에 대응.

use std::time::{Duration, Instant};

/// 측정 샘플을 모아 통계로 요약한다.
pub struct BenchAccumulator {
    samples: Vec<Duration>,
}

impl BenchAccumulator {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            samples: Vec::with_capacity(n),
        }
    }

    #[inline]
    pub fn push(&mut self, elapsed: Duration) {
        self.samples.push(elapsed);
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
        let min = self.samples[0];
        let max = self.samples[n - 1];
        let p50 = self.samples[n / 2];
        let p99 = self.samples[(n * 99 / 100).min(n - 1)];
        let avg_micros =
            self.samples.iter().map(Duration::as_secs_f64).sum::<f64>() * 1_000_000.0 / n as f64;

        let line = format!(
            "[bench {label}] n={n} avg={:.1}us p50={:.1}us p99={:.1}us min={:.1}us max={:.1}us",
            avg_micros,
            p50.as_secs_f64() * 1_000_000.0,
            p99.as_secs_f64() * 1_000_000.0,
            min.as_secs_f64() * 1_000_000.0,
            max.as_secs_f64() * 1_000_000.0,
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
    pub lazy_init: Duration,
    pub swap_chain_wait: Duration,
    pub setup: Duration,
    pub begin_clear: Duration,
    pub border: Duration,
    pub text: Duration,
    pub end_draw: Duration,
    pub present: Duration,
    pub hit_region: Duration,
}

impl PhaseRecord {
    pub fn new() -> Self {
        Self {
            lazy_init: Duration::ZERO,
            swap_chain_wait: Duration::ZERO,
            setup: Duration::ZERO,
            begin_clear: Duration::ZERO,
            border: Duration::ZERO,
            text: Duration::ZERO,
            end_draw: Duration::ZERO,
            present: Duration::ZERO,
            hit_region: Duration::ZERO,
        }
    }

    /// 직전 시점으로부터 경과 시간을 phase 필드에 누적.
    #[inline]
    pub fn add(&mut self, field: PhaseField, since: Instant) -> Instant {
        let now = Instant::now();
        let delta = now.duration_since(since);
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
        now
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
pub fn phase_now() -> Option<Instant> {
    PHASE_RECORDER.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(_) => Some(Instant::now()),
            None => None,
        }
    })
}

#[inline]
pub fn phase_record(field: PhaseField, since: Option<Instant>) -> Option<Instant> {
    PHASE_RECORDER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match (borrow.as_mut(), since) {
            (Some(rec), Some(since)) => Some(rec.add(field, since)),
            _ => None,
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

    pub fn push(&mut self, rec: &PhaseRecord, total: Duration) {
        self.lazy_init.push(rec.lazy_init);
        self.swap_chain_wait.push(rec.swap_chain_wait);
        self.setup.push(rec.setup);
        self.begin_clear.push(rec.begin_clear);
        self.border.push(rec.border);
        self.text.push(rec.text);
        self.end_draw.push(rec.end_draw);
        self.present.push(rec.present);
        self.hit_region.push(rec.hit_region);
        self.total.push(total);
    }

    pub fn report(&mut self) {
        self.total.report("paint_detailed_total");
        self.lazy_init.report("paint_detailed_lazy_init");
        self.swap_chain_wait
            .report("paint_detailed_swap_chain_wait");
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
