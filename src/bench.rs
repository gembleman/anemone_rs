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
