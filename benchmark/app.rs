//! `App::paint()` 벤치마크 실행 루프

use super::{App, bench};
use crate::bench_mem::MemSnapshot;

/// 벤치가 `paint()` 내부 블로킹(드라이버/DWM 상태 등)으로 멈추는 경우를 대비한
/// 하드 타임아웃. 데몬 스레드가 N초 후 프로세스를 강제 종료한다 — 정상 완료되면
/// 프로세스가 어차피 종료되므로 정리할 필요가 없다. (2026-08-11: MISS 벤치가
/// 드물게 종료되지 않는 사례가 있어 추가 — 재현 불가, 일회성으로 분류 중)
fn arm_bench_watchdog(seconds: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        eprintln!("[bench] watchdog: 벤치가 {seconds}초 안에 끝나지 않아 강제 종료합니다");
        std::process::exit(1);
    });
}

impl App {
    /// `paint()`를 N회 측정해 통계를 기록한다.
    pub(super) fn run_paint_bench(&mut self, iters: usize) {
        arm_bench_watchdog(90);
        // 재진입 borrow를 막기 위해 벤치 동안 클립보드 감시를 멈춘다.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            let _ = self.clipboard.stop();
        }

        // 캐시와 셰이더의 최초 비용을 제거한다.
        const WARMUP: usize = 16;
        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench warmup paint failed: {e}");
                return;
            }
        }

        let mut acc = bench::BenchAccumulator::with_capacity(iters);
        let mem_before = MemSnapshot::now();
        for _ in 0..iters {
            let started = std::time::Instant::now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench paint failed: {e}");
                if was_watching {
                    let _ = self.clipboard.start();
                }
                return;
            }
            acc.push(started.elapsed());
        }
        let mem = MemSnapshot::now().delta(mem_before);
        acc.report("paint");
        mem.report_per("paint", iters);

        if was_watching {
            let _ = self.clipboard.start();
        }
    }

    /// `paint()`의 단계별 비용을 N회 측정한다.
    /// Hook 오버헤드 때문에 `total`은 일반 벤치보다 약간 클 수 있다.
    pub(super) fn run_paint_bench_detailed(&mut self, iters: usize) {
        arm_bench_watchdog(90);
        const WARMUP: usize = 16;

        // 재진입 borrow를 막기 위해 벤치 동안 클립보드 감시를 멈춘다.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            let _ = self.clipboard.stop();
        }

        // 선택적으로 outline/shadow를 꺼 본문 비용을 분리한다.
        let no_outline = bench::bench_disable_outline();
        let saved_style = if no_outline {
            let cfg = &mut self.model.config;
            let original = cfg.translation_style.clone();
            cfg.translation_style.outline1_size = 0;
            cfg.translation_style.outline2_size = 0;
            cfg.translation_style.shadow_enabled = false;
            Some(original)
        } else {
            None
        };

        // 선택적으로 매회 캐시 key를 바꿔 miss 비용을 측정한다.
        let force_cache_miss = bench::bench_force_cache_miss();
        let saved_text = if force_cache_miss {
            Some(self.model.runtime.translated_text.clone())
        } else {
            None
        };

        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed warmup paint failed: {e}");
                if let Some(s) = saved_style {
                    self.model.config.translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.model.runtime.translated_text = t;
                }
                if was_watching {
                    let _ = self.clipboard.start();
                }
                return;
            }
        }

        let mut phased = bench::PhasedBenchAccumulator::with_capacity(iters);
        let mem_before = MemSnapshot::now();
        for i in 0..iters {
            // 접미사만 바꿔 layout 변화는 줄이고 cache miss를 만든다.
            if let Some(orig) = saved_text.as_ref() {
                self.model.runtime.translated_text = format!("{orig}#{i}");
            }
            bench::phase_begin();
            let started = std::time::Instant::now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed paint failed: {e}");
                bench::phase_end(); // 슬롯 비워서 다음 측정 안전
                if let Some(s) = saved_style {
                    self.model.config.translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.model.runtime.translated_text = t;
                }
                if was_watching {
                    let _ = self.clipboard.start();
                }
                return;
            }
            if let Some(rec) = bench::phase_end() {
                phased.push(&rec, started.elapsed());
            }
        }
        let mem = MemSnapshot::now().delta(mem_before);
        phased.report();
        mem.report_per("paint_detailed", iters);

        if let Some(s) = saved_style {
            self.model.config.translation_style = s;
        }
        if let Some(t) = saved_text {
            self.model.runtime.translated_text = t;
        }
        if was_watching {
            let _ = self.clipboard.start();
        }
    }
}
