//! 자동 확인을 걸지 판단하는 순수 로직과 `last_update_check` 갱신 정책.
//!
//! 스레드/네트워크가 필요 없는 부분만 모아 두어 단위 테스트로 고정한다. Win32,
//! config 저장, 워커 배선은 `src/app/update.rs`가 담당한다.

use super::UpdateError;
use super::check::UpdateCheck;

/// 시작 시 자동 확인을 걸어도 되는지 판단한다.
///
/// `now`와 `last_check`는 Unix epoch 초다. 마지막 확인이 없었다면(`last_check == 0`)
/// 항상 확인한다.
pub(crate) fn should_auto_check(
    enabled: bool,
    last_check: i64,
    now: i64,
    interval_secs: i64,
) -> bool {
    if !enabled {
        return false;
    }
    if last_check <= 0 {
        return true;
    }
    // now가 last_check보다 과거인 비정상 상황(시계 역행)도 안전하게 "경과 안 함"으로
    // 취급한다. saturating_sub이 음수를 0으로 접어 준다.
    now.saturating_sub(last_check) >= interval_secs
}

/// 24시간을 초 단위로.
pub(crate) const AUTO_CHECK_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// 업데이트 확인 결과에 따라 `last_update_check`를 갱신해야 하는지 판단한다.
///
/// 갱신 정책(호출자 스펙):
/// - 최신 / 새 버전 발견: O — 정상 결과이므로 갱신한다.
/// - 403 rate limit: O — 지금 재시도해도 무의미하고 요청을 더 보내면 안 된다.
/// - 네트워크 실패/타임아웃: X — 오프라인이었던 하루 때문에 24시간을 더
///   놓치면 안 된다.
/// - 파싱 실패 / asset 없음(`Unsupported`도 포함): O — 재시도해도 같은 결과다.
pub(crate) fn should_update_last_check(result: &Result<UpdateCheck, UpdateError>) -> bool {
    match result {
        Ok(_) => true,
        Err(UpdateError::Network(_)) => false,
        Err(_) => true,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/update/schedule.rs"]
mod tests;
