use super::*;

use crate::update::download::DownloadProgress;

fn progress(received: u64, total: Option<u64>) -> DownloadProgress {
    DownloadProgress { received, total }
}

#[test]
fn size_below_one_megabyte_is_shown_in_kilobytes() {
    assert_eq!(format_size(0), "0 KB");
    assert_eq!(format_size(1024), "1 KB");
    // 올림한다. 512바이트가 "0 KB"로 보이면 멈춘 것처럼 읽힌다.
    assert_eq!(format_size(512), "1 KB");
}

#[test]
fn size_at_or_above_one_megabyte_is_shown_in_megabytes() {
    assert_eq!(format_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_size(5_400_000), "5.1 MB");
}

#[test]
fn progress_with_known_total_shows_percent_and_both_sizes() {
    let text = download_progress_text(progress(2 * 1024 * 1024, Some(4 * 1024 * 1024)));
    assert_eq!(text, "다운로드 중... 50% (2.0 MB / 4.0 MB)");
}

#[test]
fn progress_without_total_shows_received_only() {
    let text = download_progress_text(progress(2 * 1024 * 1024, None));
    assert_eq!(text, "다운로드 중... (2.0 MB)");
}

/// Content-Length가 0이면 나눗셈이 NaN이 된다. 총량 미상과 같게 다뤄야 한다.
#[test]
fn zero_total_falls_back_to_received_only() {
    let text = download_progress_text(progress(0, Some(0)));
    assert_eq!(text, "다운로드 중... (0 KB)");
}

/// 서버가 실제 본문보다 작은 Content-Length를 준 경우에도 100%를 넘지 않는다.
#[test]
fn percent_is_clamped_to_one_hundred() {
    let text = download_progress_text(progress(3 * 1024 * 1024, Some(2 * 1024 * 1024)));
    assert!(
        text.starts_with("다운로드 중... 100%"),
        "예상 밖 문구: {text}"
    );
}
