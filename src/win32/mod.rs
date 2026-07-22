//! 여러 Win32 어댑터가 공유하는 작은 변환 도구.

mod strings;

pub(crate) use strings::to_wide;
