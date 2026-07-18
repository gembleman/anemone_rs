use super::should_persist_trackbar;
use windows::Win32::UI::Controls::{TB_ENDTRACK, TB_LINEDOWN, TB_THUMBTRACK};

#[test]
fn persists_trackbar_only_when_tracking_ends() {
    assert!(!should_persist_trackbar(TB_THUMBTRACK));
    assert!(!should_persist_trackbar(TB_LINEDOWN));
    assert!(should_persist_trackbar(TB_ENDTRACK));
}
