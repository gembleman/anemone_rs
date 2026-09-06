use super::*;

// `EZTRANS_ACTOR`/`EZTRANS_INITIALIZATION_GATE`는 프로세스 전역 `OnceLock`이라 이 파일의
// 테스트끼리, 그리고 `translation::settings` 쪽에서 `PreparedJob::prepare()`를 거쳐 실행되는
// 다른 테스트와도 상태를 공유한다. 실제 EzTrans64 자산이 없는 이 환경에서는 초기화가 항상
// 실패하므로 `state.engine`이 `None`으로 고정된다는 사실에 기대어 검증한다.

#[test]
fn prepare_eztrans_reports_a_missing_flat_dictionary_instead_of_panicking() {
    let result = prepare_eztrans(
        "C:/definitely/missing/JisJK.flat.bin",
        "C:/definitely/missing/Ehnd",
    );
    let error = result.expect_err("사전 파일이 없으므로 초기화가 실패해야 한다");
    assert!(
        error.contains("JisJK.flat.bin") || error.contains("평면 사전"),
        "error: {error}"
    );
}

#[test]
fn translate_with_eztrans_reports_engine_not_initialized_before_any_successful_prepare() {
    // 이 프로세스에는 실제 EzTrans64 자산이 없어 `prepare_eztrans`가 성공한 적이 없다.
    // 따라서 전역 상태의 `engine`은 항상 `None`이고, 번역 시도는 초기화 필요 오류를 반환한다.
    let result = translate_with_eztrans("こんにちは", Language::Jpn, Language::Kor);
    assert!(matches!(
        result,
        Err(TranslationError::EngineNotInitialized("EzTrans"))
    ));
}

#[test]
fn the_initialization_gate_can_be_locked_and_released_from_multiple_callers() {
    // GUI actor와 파일 번역 워커 풀이 공유하는 직렬화 게이트가 재진입 없이 정상 동작하는지 확인한다.
    {
        let _guard = eztrans_initialization_gate().lock().unwrap();
    }
    let _guard = eztrans_initialization_gate().lock().unwrap();
}
