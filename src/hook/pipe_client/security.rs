//! 파이프/이벤트 생성에 쓰는 이름 헬퍼와 보안 속성.

use std::io;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

pub(super) fn pipe_name(prefix: &str, pid: u32) -> Vec<u16> {
    format!("{prefix}{pid}").encode_utf16().chain([0]).collect()
}

/// 주입 DLL이 게임의 무결성 수준과 관계없이 호스트의 명명 오브젝트를 열 수
/// 있게 하는 보안 속성. lunactl에서 실제 게임 접속에 사용하는 것과 같다.
/// PID별 이름과 FILE_FLAG_FIRST_PIPE_INSTANCE가 다른 프로세스의 선점을 막는다.
pub(super) struct PipeSecurity {
    descriptor: *mut core::ffi::c_void,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    pub(super) fn permissive() -> Result<Self, io::Error> {
        const SDDL: &str = "D:(A;;GA;;;WD)S:(ML;;NW;;;LW)";
        let wide: Vec<u16> = SDDL.encode_utf16().chain([0]).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: wide는 NUL 종결 UTF-16이고 descriptor는 결과 포인터를 받을
        // 유효한 슬롯이다. 성공한 메모리는 Drop에서 LocalFree한다.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    pub(super) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }

    pub(super) fn as_mut_ptr(&mut self) -> *mut SECURITY_ATTRIBUTES {
        &mut self.attributes
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: descriptor는 변환 API가 LocalAlloc으로 할당해 이 객체가
        // 소유하며, 여기서 정확히 한 번 해제한다.
        unsafe {
            let _ = LocalFree(self.descriptor);
        }
    }
}
