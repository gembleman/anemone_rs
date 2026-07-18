use super::*;

#[test]
fn composition_retry_uses_bounded_exponential_backoff() {
    assert_eq!(composition_retry_delay_ms(0), 250);
    assert_eq!(composition_retry_delay_ms(1), 500);
    assert_eq!(composition_retry_delay_ms(5), 8_000);
    assert_eq!(composition_retry_delay_ms(100), 8_000);
}
