//! GUI 쪽 업데이트 배선: 시작 시 자동 확인, 수동 확인, 다운로드·적용·재시작.
//!
//! 순수 판단 로직(24시간 경과, 오류별 `last_update_check` 갱신 여부)은
//! `src/update/schedule.rs`에 있다. 여기서는 그 결과를 Win32/config/AppAction과
//! 연결한다.
//!
//! ## 자동 확인과 수동 확인의 UI 정책
//!
//! 자동 확인(시작 시)이 실패해도 화면에는 아무것도 띄우지 않는다. 사용자가
//! 요청하지 않은 작업의 실패로 방해하지 않기 위해서다 — `tracing::warn!`만
//! 남긴다. 반대로 수동 확인(설정 창의 "업데이트 확인" 버튼)이 실패하면 정보
//! 탭 상태 텍스트에 원인별 한국어 메시지를 보여준다. 이 구분은
//! `UpdateRequest::Check`에 실려 오는 [`CheckTrigger`]로 판단한다.
//!
//! ## 재시작 순서
//!
//! 적용을 확정하면 즉시 새 프로세스를 띄우지 않는다. 새 프로세스가 config를
//! 읽는 동안 기존 프로세스가 아직 실행 중이면 `AppCleanupGuard::drop`이 종료
//! 시 config를 저장해 두 프로세스가 `config.toml`/`translation_cache.sqlite3`를
//! 동시에 만질 수 있다. 그래서:
//!
//! 1. `WM_CLOSE`를 게시해 기존 종료 경로(`AppCleanupGuard::drop` →
//!    `config.save()` → `AppServices::shutdown()`)를 그대로 밟는다.
//! 2. `App::run()`이 반환한 뒤 `src/lib.rs`가 재시작 플래그를 보고 새 exe를
//!    띄운다. 인자는 붙이지 않는다(`src/cli/mod.rs`가 인자 1개 초과를 CLI 모드로
//!    분기하므로 GUI로 진입하지 못한다).

use std::cell::Cell;

use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW,
        PostMessageW as PostMessageWSys, WM_CLOSE,
    },
};

use crate::dialogs::SettingsDialog;
use crate::update::check::UpdateCheck;
use crate::update::schedule::{
    AUTO_CHECK_INTERVAL_SECS, should_auto_check, should_update_last_check,
};
use crate::update::worker::{CheckTrigger, UpdateOutcome};
use crate::update::{UpdateError, Version};

use super::{App, state};

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// 자동 업데이트 네트워크 호출을 테스트/검증 실행에서 차단한다.
///
/// 일반 사용자는 `update_check_enabled` 설정으로 제어한다. `cfg!(test)`는
/// cargo 테스트가 GUI 시작 경로를 직접 밟는 경우에도 네트워크를 만들지 않게
/// 하고, `ANEMONE_DISABLE_AUTO_UPDATE`는 release 바이너리를 구동하는 GUI E2E가
/// 같은 보장을 갖도록 한다.
fn auto_update_disabled_for_test() -> bool {
    cfg!(test) || std::env::var_os("ANEMONE_DISABLE_AUTO_UPDATE").is_some()
}

thread_local! {
    /// `App::run()`이 반환한 뒤 `src/lib.rs`가 읽어가는 재시작 요청.
    /// 프로세스마다 한 번만 존재하는 UI 스레드에서만 쓰이므로 thread_local로 충분하다.
    static RESTART_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

/// `App::run()`이 반환한 뒤 `src/lib.rs`가 호출한다. 적용 도중 재시작이
/// 요청됐으면 `true`.
pub fn take_restart_requested() -> bool {
    RESTART_REQUESTED.with(|flag| flag.replace(false))
}

/// 수동 확인 실패를 사용자에게 보여줄 짧은 한국어 문구로 바꾼다.
///
/// `UpdateError`의 `Display`는 진단용 원인을 그대로 담고 있어(`Network`의 경우
/// 괄호 안에 원문 오류가 붙는다) UI에 그대로 노출하면 지저분하다. 여기서는
/// 사용자가 다음에 뭘 해야 하는지 알 수 있는 짧은 문장만 남긴다.
fn manual_check_failure_text(error: &UpdateError) -> String {
    match error {
        UpdateError::Network(_) => {
            "업데이트 서버에 연결할 수 없습니다. 인터넷 연결을 확인해주세요.".to_string()
        }
        UpdateError::RateLimited => {
            "확인 요청이 많아 잠시 제한되었습니다. 1시간 뒤 다시 시도해주세요.".to_string()
        }
        UpdateError::Api { code } => format!("업데이트 서버가 오류를 반환했습니다 (HTTP {code})"),
        UpdateError::Parse(_) => "업데이트 정보를 해석할 수 없습니다.".to_string(),
        UpdateError::UntrustedUrl(_) => "허용되지 않은 주소로 연결을 시도했습니다.".to_string(),
        UpdateError::TooLarge { .. } => "내려받을 데이터가 너무 큽니다.".to_string(),
        UpdateError::ChecksumMismatch => {
            "내려받은 파일이 손상되었습니다. 다시 시도해주세요.".to_string()
        }
        UpdateError::Hash(_) => "해시를 계산할 수 없습니다.".to_string(),
        UpdateError::NotWritable(_) => {
            "이 폴더에 쓸 수 없어 자동 업데이트를 적용할 수 없습니다. 릴리스 페이지에서 직접 내려받아주세요.".to_string()
        }
        UpdateError::RollbackFailed { .. } => error.to_string(),
        UpdateError::Io(_) => "파일을 저장할 수 없습니다.".to_string(),
    }
}

/// 바이트 수를 사람이 읽는 크기로 바꾼다.
///
/// MB 미만은 KB로, 그 이상은 소수 첫째 자리까지의 MB로 보여준다. 업데이트 exe는
/// 수 MB 수준이라 GB 단위는 다루지 않는다.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

/// 다운로드 진행도를 정보 탭 상태 텍스트 한 줄로 만든다.
///
/// 총량을 아는 경우에만 퍼센트를 낸다. GitHub은 사실상 항상 Content-Length를
/// 주지만, 없을 때 0%에 멈춘 것처럼 보이느니 받은 용량만 보여주는 편이 낫다.
fn download_progress_text(progress: crate::update::download::DownloadProgress) -> String {
    match progress.total {
        // 총량이 0이면 나눗셈이 무의미하다. 빈 asset은 정상 릴리스에서 나올 수
        // 없지만, 여기서 0으로 나눠 NaN을 보여주면 원인 파악이 어려워진다.
        Some(total) if total > 0 => {
            let percent = (progress.received as f64 / total as f64 * 100.0).min(100.0);
            format!(
                "다운로드 중... {percent:.0}% ({} / {})",
                format_size(progress.received),
                format_size(total)
            )
        }
        _ => format!("다운로드 중... ({})", format_size(progress.received)),
    }
}

impl App {
    /// GUI 진입 시점에 한 번 호출한다. 조건을 통과하면 확인 요청만 보내고 즉시
    /// 반환한다 — 시작을 지연시키지 않는다.
    pub(super) fn maybe_start_auto_update_check(&self) {
        if auto_update_disabled_for_test() {
            tracing::debug!("테스트/검증 실행에서는 자동 업데이트 확인을 건너뜁니다");
            return;
        }
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
        self.services
            .request_update_check(Version::current(), CheckTrigger::Auto);
    }

    /// 설정 창의 "업데이트 확인" 버튼이 눌렸을 때 호출된다.
    pub(super) fn start_manual_update_check(&mut self) {
        if self.update_operation_in_progress {
            return;
        }
        self.update_operation_in_progress = true;
        SettingsDialog::set_update_available(false);
        SettingsDialog::set_update_check_button_enabled(false);
        SettingsDialog::update_status_text("확인 중...");
        self.services
            .request_update_check(Version::current(), CheckTrigger::Manual);
    }

    /// 설정 창에서 발견한 업데이트의 다운로드·적용을 요청했을 때 호출된다.
    pub(super) fn start_update_apply(&mut self) {
        if self.update_operation_in_progress {
            return;
        }
        let Some(update) = self.pending_update.clone() else {
            return;
        };

        // 파일 번역이 진행 중이면 적용을 거부한다. helper 자동 재시작이
        // `current_exe()`로 자기 자신을 띄우는데, 교체 도중이면 실패한다.
        if crate::dialogs::file_trans_progress::FileTransProgressDialog::is_running() {
            SettingsDialog::update_status_text(
                "파일 번역이 진행 중입니다. 완료한 뒤 다시 시도해주세요.",
            );
            return;
        }

        let confirmed = unsafe {
            let message: Vec<u16> = format!(
                "새 버전 {}을(를) 내려받아 적용할까요?\n적용 후 아네모네가 자동으로 재시작됩니다.",
                update.version
            )
            .encode_utf16()
            .chain([0])
            .collect();
            let caption: Vec<u16> = "업데이트 적용".encode_utf16().chain([0]).collect();
            MessageBoxW(
                self.hwnd,
                message.as_ptr(),
                caption.as_ptr(),
                MB_ICONQUESTION | MB_YESNO,
            )
        };
        if confirmed != IDYES {
            return;
        }

        self.update_operation_in_progress = true;
        self.pending_update = None;
        SettingsDialog::set_update_available(false);
        SettingsDialog::set_update_check_button_enabled(false);
        SettingsDialog::update_status_text("다운로드 중...");

        let Ok(current_exe) = std::env::current_exe() else {
            tracing::error!("실행 파일 경로를 확인할 수 없어 업데이트를 적용할 수 없습니다");
            self.update_operation_in_progress = false;
            SettingsDialog::set_update_check_button_enabled(true);
            SettingsDialog::update_status_text("실행 파일 경로를 확인할 수 없습니다.");
            return;
        };
        // 교체 대상 exe와 같은 볼륨/폴더에 받아야 이후 교체가 rename으로 끝난다.
        let destination = current_exe.with_extension("exe.download");

        self.services.request_update_download(update, destination);
    }

    /// `WM_UPDATE_PROGRESS` 수신 시 호출된다. 슬롯의 최신 진행도만 읽어 상태
    /// 텍스트를 갱신한다.
    ///
    /// 알림이 밀려 다운로드가 끝난 뒤 도착할 수 있다. 그때 진행도를 덮어쓰면
    /// "적용 완료..." 같은 최종 문구가 "다운로드 중..."으로 되돌아가므로,
    /// 작업이 진행 중일 때만 그린다.
    pub(super) fn handle_update_progress(&mut self) {
        if !self.update_operation_in_progress {
            return;
        }
        let progress = self.services.update_download_progress();
        SettingsDialog::update_status_text(&download_progress_text(progress));
    }

    /// `WM_UPDATE_RESULT` 수신 시 호출된다. 워커가 채워 둔 결과를 모두 꺼내 처리한다.
    pub(super) fn handle_update_result(&mut self) {
        let outcomes = self.services.drain_update_results();
        for outcome in outcomes {
            match outcome {
                UpdateOutcome::Check { result, trigger } => {
                    self.handle_update_check_result(result, trigger);
                }
                UpdateOutcome::Download(result) => self.handle_update_download_result(result),
            }
        }
    }

    fn handle_update_check_result(
        &mut self,
        result: Result<UpdateCheck, UpdateError>,
        trigger: CheckTrigger,
    ) {
        // 정책에 따라 last_update_check 갱신 여부를 결정한다. 네트워크 실패는
        // 갱신하지 않는다 — 갱신하면 오프라인이었던 하루 때문에 다음 24시간을
        // 더 놓친다.
        let new_last_check = should_update_last_check(&result).then(now_unix);
        let effects = self
            .model
            .update(state::AppAction::UpdateCheckSettled(new_last_check));
        self.run_effects(effects);

        if trigger == CheckTrigger::Manual {
            self.update_operation_in_progress = false;
            SettingsDialog::set_update_check_button_enabled(true);
        }

        match result {
            Ok(UpdateCheck::UpToDate) => {
                tracing::debug!("업데이트 확인 결과: 최신 버전입니다");
                self.pending_update = None;
                if trigger == CheckTrigger::Manual {
                    SettingsDialog::update_status_text("최신 버전입니다.");
                }
            }
            Ok(UpdateCheck::Available(update)) => {
                tracing::info!("새 버전을 사용할 수 있습니다: {}", update.version);
                let version = update.version.clone();
                self.pending_update = Some(update);
                if trigger == CheckTrigger::Manual {
                    SettingsDialog::set_update_available(true);
                    SettingsDialog::update_status_text(&format!(
                        "새 버전 v{version}을 사용할 수 있습니다. \"지금 업데이트\"를 누르면 내려받아 적용합니다."
                    ));
                }
            }
            Ok(UpdateCheck::Unsupported {
                version,
                reason,
                release_page_url,
            }) => {
                tracing::warn!(
                    "새 버전 {version}이(가) 있지만 자동 업데이트를 지원하지 않습니다: \
                     {reason} (수동 내려받기: {release_page_url})"
                );
                self.pending_update = None;
                if trigger == CheckTrigger::Manual {
                    SettingsDialog::update_status_text(&format!(
                        "새 버전 v{version}이 있지만 자동 업데이트를 지원하지 않습니다. 릴리스 페이지에서 직접 내려받아주세요."
                    ));
                }
            }
            Err(error) => {
                tracing::warn!("업데이트 확인 실패: {error}");
                self.pending_update = None;
                if trigger == CheckTrigger::Manual {
                    SettingsDialog::update_status_text(&manual_check_failure_text(&error));
                }
            }
        }
    }

    fn handle_update_download_result(
        &mut self,
        result: Result<crate::update::download::StagedUpdate, UpdateError>,
    ) {
        self.update_operation_in_progress = false;
        SettingsDialog::set_update_check_button_enabled(true);

        match result {
            Ok(staged) => {
                tracing::info!(
                    "업데이트 파일을 내려받아 검증했습니다: {}",
                    staged.path().display()
                );
                self.apply_staged_update(staged);
            }
            Err(error) => {
                tracing::warn!("업데이트 다운로드 실패: {error}");
                SettingsDialog::update_status_text(&manual_check_failure_text(&error));
            }
        }
    }

    /// 검증까지 끝난 파일을 실행 중인 exe와 교체하고 재시작을 예약한다.
    fn apply_staged_update(&mut self, staged: crate::update::download::StagedUpdate) {
        let Ok(current_exe) = std::env::current_exe() else {
            tracing::error!("실행 파일 경로를 확인할 수 없어 업데이트를 적용할 수 없습니다");
            SettingsDialog::update_status_text("실행 파일 경로를 확인할 수 없습니다.");
            return;
        };

        let staged_path = staged.into_applied();
        match crate::update::apply::replace_running_executable(&current_exe, &staged_path) {
            Ok(crate::update::apply::Applied::ReadyToRestart) => {
                tracing::info!("업데이트를 적용했습니다. 재시작합니다.");
                SettingsDialog::update_status_text("적용 완료. 재시작합니다...");
                RESTART_REQUESTED.with(|flag| flag.set(true));
                // SAFETY: self.hwnd는 App이 소유한 유효한 창이다. 종료 경로 진입만
                // 요청하고 여기서 직접 파괴하지 않는다 — 기존 WM_CLOSE 처리
                // (AppCleanupGuard::drop → config.save() → AppServices::shutdown())를
                // 그대로 밟게 하기 위해서다.
                if unsafe { PostMessageWSys(self.hwnd, WM_CLOSE, 0, 0) } == 0 {
                    tracing::error!("업데이트 적용 후 종료 요청(WM_CLOSE) 게시 실패");
                    RESTART_REQUESTED.with(|flag| flag.set(false));
                }
            }
            Err(UpdateError::RollbackFailed { backup, cause }) => {
                // 이 경우 다음 실행에서 앱이 뜨지 않는다. 로그로는 부족하므로
                // 백업 경로 전문을 MessageBoxW로 직접 보여준다.
                tracing::error!(
                    "업데이트 롤백 실패. 실행 파일이 {}에 남아 있습니다: {cause}",
                    backup.display()
                );
                let message: Vec<u16> = format!(
                    "업데이트에 실패했고 이전 버전으로 되돌리지도 못했습니다.\n\n{}\n\n파일의 확장자를 .exe로 바꿔주세요.\n(원인: {cause})",
                    backup.display()
                )
                .encode_utf16()
                .chain([0])
                .collect();
                let caption: Vec<u16> = "업데이트 적용 실패".encode_utf16().chain([0]).collect();
                unsafe {
                    let _ = MessageBoxW(
                        self.hwnd,
                        message.as_ptr(),
                        caption.as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
                SettingsDialog::update_status_text("업데이트 적용에 실패했습니다.");
            }
            Err(error) => {
                tracing::error!("업데이트 적용 실패: {error}");
                SettingsDialog::update_status_text(&manual_check_failure_text(&error));
            }
        }
    }
}

/// 재시작 플래그가 켜져 있으면 인자 없이 자기 자신을 새로 띄운다.
///
/// **인자를 붙이지 않는다.** `src/cli/mod.rs`가 인자 개수 1 초과를 CLI 모드로
/// 분기하므로, 인자를 붙이면 GUI로 진입하지 못한다. `App::run()`이 반환한
/// 뒤 — 즉 기존 프로세스의 종료 경로(config 저장 포함)가 모두 끝난 뒤에만
/// 호출해야 한다.
pub fn restart_after_update_if_requested() {
    if !take_restart_requested() {
        return;
    }
    let Ok(current_exe) = std::env::current_exe() else {
        tracing::error!("재시작할 실행 파일 경로를 확인할 수 없습니다");
        return;
    };
    match std::process::Command::new(&current_exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => tracing::info!("업데이트된 실행 파일로 재시작했습니다."),
        Err(error) => tracing::error!("업데이트 후 재시작에 실패했습니다: {error}"),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/update.rs"]
mod tests;

/// 릴리스 페이지를 기본 브라우저로 연다. 자동 업데이트 경로가 막혀도 항상
/// 남아 있는 수동 탈출구다.
pub(crate) fn open_release_page(owner: HWND) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let url = format!(
        "https://github.com/{}/{}/releases",
        crate::update::GITHUB_OWNER,
        crate::update::GITHUB_REPO
    );
    let url_wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    let operation: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    // SAFETY: owner는 유효한 창 핸들이거나 무시돼도 되는 기본값이다. 나머지
    // 인자는 정적이거나 이 함수 스코프에서 살아 있는 값이다.
    let result = unsafe {
        ShellExecuteW(
            owner,
            operation.as_ptr(),
            url_wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW는 성공 시 32보다 큰 값을 반환한다.
    if (result as usize) <= 32 {
        tracing::warn!("릴리스 페이지를 열지 못했습니다 (코드 {})", result as isize);
    }
}
