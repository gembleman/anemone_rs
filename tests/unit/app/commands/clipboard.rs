use super::advance_pause_session_counter;

#[test]
fn advance_pause_session_counter_issues_the_current_value_and_increments() {
    assert_eq!(advance_pause_session_counter(1), (1, 2));
    assert_eq!(advance_pause_session_counter(41), (41, 42));
}

#[test]
fn advance_pause_session_counter_never_issues_or_lands_on_zero() {
    // u64::MAX + 1은 wrapping으로 0이 되므로, 그 경우에만 1로 건너뛰어야 한다 —
    // 0은 "일시정지 세션 없음"을 뜻하는 sentinel이기 때문이다.
    assert_eq!(advance_pause_session_counter(u64::MAX), (u64::MAX, 1));
}

#[test]
fn advance_pause_session_counter_can_still_issue_zero_as_a_starting_value() {
    // 발급되는 값 자체가 0이 될 수는 있다 (필드 초기값이 0이면). 다음 값 계산만
    // 0을 건너뛰면 된다.
    assert_eq!(advance_pause_session_counter(0), (0, 1));
}
