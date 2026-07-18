//! `App::paint()` 벤치마크 실행 루프

use super::{App, bench};

impl App {
    /// `paint()`를 N회 측정해 통계를 기록한다.
    pub(super) fn run_paint_bench(&mut self, iters: usize) {
        // 재진입 borrow를 막기 위해 벤치 동안 클립보드 감시를 멈춘다.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            self.clipboard.stop();
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
        for _ in 0..iters {
            let started = std::time::Instant::now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench paint failed: {e}");
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
            acc.push(started.elapsed());
        }
        acc.report("paint");

        if was_watching {
            self.clipboard.start();
        }
    }

    /// `paint()`의 단계별 비용을 N회 측정한다.
    /// Hook 오버헤드 때문에 `total`은 일반 벤치보다 약간 클 수 있다.
    pub(super) fn run_paint_bench_detailed(&mut self, iters: usize) {
        const WARMUP: usize = 16;

        // 재진입 borrow를 막기 위해 벤치 동안 클립보드 감시를 멈춘다.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            self.clipboard.stop();
        }

        // 선택적으로 outline/shadow를 꺼 본문 비용을 분리한다.
        let no_outline = bench::bench_disable_outline();
        let saved_style = if no_outline {
            let mut cfg = self.config.borrow_mut();
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
            Some(self.state.translated_text.clone())
        } else {
            None
        };

        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed warmup paint failed: {e}");
                if let Some(s) = saved_style {
                    self.config.borrow_mut().translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.state.translated_text = t;
                }
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
        }

        let mut phased = bench::PhasedBenchAccumulator::with_capacity(iters);
        for i in 0..iters {
            // 접미사만 바꿔 layout 변화는 줄이고 cache miss를 만든다.
            if let Some(orig) = saved_text.as_ref() {
                self.state.translated_text = format!("{}#{}", orig, i);
            }
            bench::phase_begin();
            let started = std::time::Instant::now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed paint failed: {e}");
                bench::phase_end(); // 슬롯 비워서 다음 측정 안전
                if let Some(s) = saved_style {
                    self.config.borrow_mut().translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.state.translated_text = t;
                }
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
            if let Some(rec) = bench::phase_end() {
                phased.push(&rec, started.elapsed());
            }
        }
        phased.report();

        if let Some(s) = saved_style {
            self.config.borrow_mut().translation_style = s;
        }
        if let Some(t) = saved_text {
            self.state.translated_text = t;
        }
        if was_watching {
            self.clipboard.start();
        }
    }
}
