//! LUNA_HOST/LUNA_HOOK named pipe 서버 측 구현.
//!
//! - [`server`]: 파이프 생성/접속 대기/읽기·쓰기를 담당하는 [`PipeServer`].
//! - [`handshake`]: `communication_initialize`의 호스트 측 절차.
//! - [`notification`]: hook → host 알림 wire 디코딩.
//! - [`command`]: host → hook 명령 wire 인코딩.
//! - [`overlapped`]: 오버랩(비동기) I/O 공용 헬퍼.

mod command;
mod handshake;
mod notification;
mod overlapped;
mod security;
mod server;

use std::io;

pub use command::{
    build_find_hook, build_general_search_param, build_new_hook, build_remove_hook,
    build_text_search_param,
};
pub use notification::{FoundHook, Notification, TextNotification};
pub(crate) use server::PipeServer;

/// 파이프 오류.
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error("파이프 생성 실패: {0}")]
    Create(io::Error),
    #[error("게임과의 연결이 끊겼습니다")]
    Disconnected,
    #[error("핸드셰이크 실패: {0}")]
    Handshake(String),
    #[error("DLL이 제한 시간 안에 파이프에 접속하지 않았습니다")]
    ConnectTimeout,
}
