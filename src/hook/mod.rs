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

pub use worker::{HookEvent, HookRequest, HookWorker};

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
