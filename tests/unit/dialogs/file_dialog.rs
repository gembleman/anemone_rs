use super::show_was_accepted;
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::core::{Error, HRESULT};

#[test]
fn distinguishes_user_cancel_from_com_failure() {
    let cancelled = Error::from_hresult(HRESULT::from_win32(ERROR_CANCELLED.0));
    assert!(!show_was_accepted(Err(cancelled)).unwrap());

    let failure = Error::from_hresult(HRESULT(0x80004005u32 as i32));
    assert!(show_was_accepted(Err(failure)).is_err());
}
