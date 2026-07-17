//! `App::paint()` 벤치마크 실행 루프

use super::{App, bench};

impl App {
    /// paint() 1 회 비용을 N 회 반복 측정해 통계를 로그로 출력.
    ///
    /// `ANEMONE_BENCH_PAINT=<N>` 환경변수가 설정된 경우 초기 paint 직후 1 회
    /// 호출된다. D2D 합성 경로 (DCRenderTarget → HwndRT/DComp) 재작성 결정의
    /// baseline 측정용.
    pub(super) fn run_paint_bench(&mut self, iters: usize) {
        // 벤치 루프 중 클립보드 이벤트가 re-entrant borrow 를 유발해 패닉하는 것을
        // 방지한다. 루프 종료 후 원래 상태로 복구.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            self.clipboard.stop();
        }

        // 워밍업 (캐시 / 셰이더 컴파일 등의 1 회성 비용 제거)
        const WARMUP: usize = 16;
        for _ in 0..WARMUP {
            if let Err(e) = self.paint() {
                tracing::warn!("bench warmup paint failed: {e}");
                return;
            }
        }

        let mut acc = bench::BenchAccumulator::with_capacity(iters);
        for _ in 0..iters {
            let t0 = acc.timer().now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench paint failed: {e}");
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
            let t1 = acc.timer().now();
            acc.push(t1 - t0);
        }
        acc.report("paint");

        if was_watching {
            self.clipboard.start();
        }
    }

    /// paint() 의 phase 별 비용을 분리 측정. `ANEMONE_BENCH_PAINT_DETAILED=<N>`
    /// 환경변수가 설정된 경우 초기 paint 직후 1 회 호출된다.
    ///
    /// 결과 라벨: `paint_detailed_<phase>` — `setup`, `begin_clear`, `border`,
    /// `text`, `end_draw`, `present`, `hit_region`, `lazy_init`, `total`.
    /// Present 경로 최적화 작업의 ROI 판단 (어느 phase 가 floor 를 만드는가)
    /// 용도. paint() 내부 hook 의 thread_local borrow overhead 가 있어 total
    /// 자체는 일반 `paint` 벤치보다 약간 더 느릴 수 있다.
    pub(super) fn run_paint_bench_detailed(&mut self, iters: usize) {
        const WARMUP: usize = 16;

        // 벤치 루프 중 클립보드 이벤트가 re-entrant borrow 를 유발해 패닉하는 것을
        // 방지한다. 루프 종료 후 원래 상태로 복구.
        let was_watching = self.clipboard.is_watching();
        if was_watching {
            self.clipboard.stop();
        }

        // `ANEMONE_BENCH_PAINT_NO_OUTLINE=1` 토글 — text/end_draw 비용의 출처가
        // outline/shadow geometry 인지 텍스트 본문인지 분리 측정. config 를
        // 임시로 수정하고 측정 후 원복한다 (production paint 경로는 무변경).
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

        // `ANEMONE_BENCH_PAINT_CACHE_MISS=1` 토글 — 매 iteration 마다
        // current_text 끝에 카운터를 붙여 outline 비트맵 / layout /
        // geometry 캐시를 강제 miss 시킨다. 항목 10 의 "miss 폭주" 위험 실측용.
        let force_cache_miss = bench::bench_force_cache_miss();
        let saved_text = if force_cache_miss {
            Some(self.current_text.clone())
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
                    self.current_text = t;
                }
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
        }

        let mut phased = bench::PhasedBenchAccumulator::with_capacity(iters);
        let outer_timer = bench::QpcTimer::new();
        for i in 0..iters {
            // 매 iteration 마다 텍스트 변경 → 캐시 miss 강제.
            // 카운터는 텍스트 끝 ("…#0", "#1", …) 에 붙여 layout box 크기
            // 변동을 최소화 (자릿수 1 → 2 → 3 자리 전환점에서만 폭 변화).
            if let Some(orig) = saved_text.as_ref() {
                self.current_text = format!("{}#{}", orig, i);
            }
            bench::phase_begin();
            let t0 = outer_timer.now();
            if let Err(e) = self.paint() {
                tracing::warn!("bench detailed paint failed: {e}");
                bench::phase_end(); // 슬롯 비워서 다음 측정 안전
                if let Some(s) = saved_style {
                    self.config.borrow_mut().translation_style = s;
                }
                if let Some(t) = saved_text {
                    self.current_text = t;
                }
                if was_watching {
                    self.clipboard.start();
                }
                return;
            }
            let t1 = outer_timer.now();
            if let Some(rec) = bench::phase_end() {
                phased.push(&rec, t1 - t0);
            }
        }
        phased.report();

        if let Some(s) = saved_style {
            self.config.borrow_mut().translation_style = s;
        }
        if let Some(t) = saved_text {
            self.current_text = t;
        }
        if was_watching {
            self.clipboard.start();
        }
    }
}
