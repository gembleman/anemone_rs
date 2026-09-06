//! 게임 텍스트 후킹 통합 (lunahook_rs 기반).
//!
//! 구성은 `src/update/worker.rs`와 동일한 스레드 모델이다:
//!
//! - UI 스레드는 절대 블로킹하지 않는다. 요청(attach/detach/후크 명령)은
//!   채널로 워커에 전달되고, 결과는 공유 이벤트 슬롯에 쌓였다가
//!   `PostMessageW`로 깨어난 UI 스레드가 꺼내 간다.
//! - attach 요청을 받으면 세션 스레드를 하나 더 띄운다. 세션 스레드는
//!   인젝션 → named pipe 서버 생성 → 핸드셰이크 → 알림 읽기 루프까지
//!   전담하며, 파이프가 끊기거나 detach 요청을 받으면 끝난다.
//! - 단일 게임 정책: 새 attach가 오면 기존 세션을 먼저 정리한다.
//!
//! 파이프 프로토콜(wire 형식, 핸드셰이크 순서)은 lunahook_rs::protocol의
//! 타입을 그대로 사용한다 — anemone과 주입 DLL이 같은 소스에서 타입을
//! 공유하므로 프로토콜 드리프트가 생기지 않는다.

pub mod inject;
pub mod pipe_client;
pub mod process_list;
pub mod text_bridge;
mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

pub use worker::{HookEvent, HookRequest, HookWorker};

use text_bridge::HookSource;

/// 현재 후킹 세션 활성 여부의 스냅샷. App이 Attached/Detached 이벤트를 처리할 때
/// 갱신하고, 설정 다이얼로그처럼 같은 UI 스레드에서 동기 조회가 필요한 곳이 읽는다.
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 후킹 세션 활성 스냅샷을 갱신한다. (UI 스레드 전용)
pub(crate) fn set_session_active(active: bool) {
    SESSION_ACTIVE.store(active, Ordering::Release);
}

/// 현재 후킹 세션이 살아 있는지. (UI 스레드에서 동기 조회)
pub(crate) fn session_active() -> bool {
    SESSION_ACTIVE.load(Ordering::Acquire)
}

/// 사용자가 고른 텍스트 스레드의 스냅샷. `None`이면 아직 고르지 않았다.
///
/// 선택 자체는 후킹 관리 창의 thread_local이 들고 있지만 그것은 UI 스레드에서만
/// 보인다. 후킹 워커 스레드가 디버그 로그를 남길지 정하려면 같은 값을 자기
/// 스레드에서도 읽을 수 있어야 해서 여기에 함께 게시한다.
static SELECTED_TEXT_SOURCE: RwLock<Option<HookSource>> = RwLock::new(None);

/// 선택된 텍스트 스레드 스냅샷을 갱신한다. (UI 스레드 전용)
pub(crate) fn set_selected_text_source(source: Option<HookSource>) {
    *SELECTED_TEXT_SOURCE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = source;
}

/// 이 출처가 사용자가 고른 텍스트 스레드인가. 아직 고르지 않았으면 거짓이다.
///
/// 텍스트 이벤트마다 불린다 — 글자 단위 훅이면 초당 수천 번이다. 읽기 잠금은
/// 경합이 없을 때 값 하나 읽는 것과 다르지 않고, 뒤따르는 로그 한 줄을 아예
/// 쓰지 않게 되므로 전체로는 오히려 싸다.
pub(crate) fn is_selected_text_source(source: HookSource) -> bool {
    *SELECTED_TEXT_SOURCE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == Some(source)
}

/// 현재 후킹 대상 게임 프로세스의 신원. 번역 요청 출처 입증에 쓰인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// 실행 파일 이름 (예: `game.exe`).
    pub name: String,
    /// 실행 파일 SHA-256(소문자 16진 64자). 파일을 읽지 못했으면 `None`.
    pub sha256: Option<String>,
}

/// 활성 세션의 프로세스 신원 스냅샷. UI 스레드가 Attached/Detached 이벤트로
/// 갱신하고, 번역 워커(번역 서버 요청 직렬화)가 잠깐 잠그고 읽는다.
static SESSION_IDENTITY: Mutex<Option<ProcessIdentity>> = Mutex::new(None);

/// 세션 프로세스 신원 스냅샷을 갱신한다. (`None`으로 지우면 세션 종료)
pub(crate) fn set_session_identity(identity: Option<ProcessIdentity>) {
    *SESSION_IDENTITY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = identity;
}

/// 현재 후킹된 게임 프로세스 신원. 세션이 없으면 `None`.
#[cfg_attr(not(mys_private), allow(dead_code))]
pub fn session_identity() -> Option<ProcessIdentity> {
    SESSION_IDENTITY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// pid의 실행 파일을 읽어 SHA-256 다이제스트를 계산한다.
///
/// attach 흐름(hook 워커 스레드)에서 한 번만 호출한다. 수백 MB 실행 파일도
/// CNG 해시로 1초 안에 처리되며, 실패해도 치명적이지 않다 — 해시 없이
/// 이름만이라도 서버로 보내는 편이 낫다.
fn compute_exe_digest(pid: u32) -> Option<String> {
    use std::io::Read;

    let path = process_list::process_image_full_path(pid)?;
    let file = std::fs::File::open(&path).ok()?;
    let mut hasher = crate::update::sha256::Hasher::new().ok()?;
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => hasher.update(&chunk[..read]).ok()?,
            Err(_) => return None,
        }
    }
    Some(hasher.finish().ok()?.to_string())
}

/// 대상 프로세스의 비트니스.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    X86,
}

impl Arch {
    /// DLL이 놓이는 하위 폴더 이름 (`hook/x64`, `hook/x86`).
    pub fn folder(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::X86 => "x86",
        }
    }

    /// 표시용 이름.
    pub fn label(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::X86 => "x86",
        }
    }
}

/// 실행 파일 옆 `hook/<arch>/` 폴더의 절대 경로.
pub(crate) fn hook_dir(arch: Arch) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    exe_dir.join("hook").join(arch.folder())
}

/// 주입할 DLL 경로.
pub(crate) fn dll_path(arch: Arch) -> PathBuf {
    hook_dir(arch).join(match arch {
        Arch::X64 => "lunahook_rs64.dll",
        Arch::X86 => "lunahook_rs32.dll",
    })
}

/// x86 대상 인젝터 헬퍼 실행 파일 경로 (`tools/inject32` 빌드 산출물).
pub(crate) fn inject32_helper_path() -> PathBuf {
    hook_dir(Arch::X86).join("anemone_inject32.exe")
}

#[cfg(test)]
#[path = "../../tests/unit/hook/mod.rs"]
mod tests;
