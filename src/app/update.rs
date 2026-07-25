//! GUI 쪽 업데이트 배선: 시작 시 자동 확인과 워커 결과 처리.
//!
//! 순수 판단 로직(24시간 경과, 오류별 `last_update_check` 갱신 여부)은
//! `src/update/schedule.rs`에 있다. 여기서는 그 결과를 Win32/config/AppAction과
//! 연결한다.

use crate::update::check::UpdateCheck;
use crate::update::schedule::{
    AUTO_CHECK_INTERVAL_SECS, should_auto_check, should_update_last_check,
};
use crate::update::worker::UpdateOutcome;
use crate::update::{UpdateError, Version};

use super::{App, state};

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

impl App {
    /// GUI 진입 시점에 한 번 호출한다. 조건을 통과하면 확인 요청만 보내고 즉시
    /// 반환한다 — 시작을 지연시키지 않는다.
    pub(super) fn maybe_start_auto_update_check(&self) {
        let config = &self.model.config;
        if !should_auto_check(
            config.update_check_enabled,
            config.last_update_check,
            now_unix(),
            AUTO_CHECK_INTERVAL_SECS,
        ) {
            return;
        }
        tracing::debug!("자동 업데이트 확인을 시작합니다");
        self.services.request_update_check(Version::current());
    }

    /// `WM_UPDATE_RESULT` 수신 시 호출된다. 워커가 채워 둔 결과를 모두 꺼내 처리한다.
    pub(super) fn handle_update_result(&mut self) {
        let outcomes = self.services.drain_update_results();
        for outcome in outcomes {
            match outcome {
                UpdateOutcome::Check(result) => self.handle_update_check_result(result),
                UpdateOutcome::Download(result) => self.handle_update_download_result(result),
            }
        }
    }

    fn handle_update_check_result(&mut self, result: Result<UpdateCheck, UpdateError>) {
        // 정책에 따라 last_update_check 갱신 여부를 결정한다. 네트워크 실패는
        // 갱신하지 않는다 — 갱신하면 오프라인이었던 하루 때문에 다음 24시간을
        // 더 놓친다.
        let new_last_check = should_update_last_check(&result).then(now_unix);
        let effects = self
            .model
            .update(state::AppAction::UpdateCheckSettled(new_last_check));
        self.run_effects(effects);

        // UI는 아직 배선되지 않았다(다음 작업). 자동 확인의 실패는 사용자가
        // 요청하지 않은 작업이므로 화면에는 아무것도 띄우지 않고 로그만 남긴다.
        match result {
            Ok(UpdateCheck::UpToDate) => {
                tracing::debug!("업데이트 확인 결과: 최신 버전입니다");
            }
            Ok(UpdateCheck::Available(update)) => {
                tracing::info!("새 버전을 사용할 수 있습니다: {}", update.version);
            }
            Ok(UpdateCheck::Unsupported { version, reason }) => {
                tracing::warn!(
                    "새 버전 {version}이(가) 있지만 자동 업데이트를 지원하지 않습니다: {reason}"
                );
            }
            Err(error) => {
                tracing::warn!("업데이트 확인 실패: {error}");
            }
        }
    }

    fn handle_update_download_result(
        &mut self,
        result: Result<crate::update::download::StagedUpdate, UpdateError>,
    ) {
        // 다운로드를 트리거하는 UI가 아직 없으므로 현재는 로그만 남긴다.
        // 적용 여부는 항상 사용자가 결정한다(모듈 문서 참고) — 이 경로가 자동으로
        // exe를 교체하지는 않는다.
        match result {
            Ok(staged) => {
                tracing::info!(
                    "업데이트 파일을 내려받아 검증했습니다: {}",
                    staged.path().display()
                );
            }
            Err(error) => {
                tracing::warn!("업데이트 다운로드 실패: {error}");
            }
        }
    }
}
