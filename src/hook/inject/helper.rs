//! 다른 비트니스 대상(x64 anemone → x86 게임)에 대한 인젝션/제거.
//!
//! WOW64 경계를 넘는 원격 스레드 생성은 불가능하므로, 32비트로 빌드된 헬퍼
//! (`tools/inject32`)를 스폰해 같은 절차를 32비트 세계에서 대신 실행한다.

use std::path::Path;

use super::InjectError;

/// x86 게임용: 32비트 헬퍼 exe를 스폰해 인젝션을 위임한다.
///
/// 헬퍼는 `<helper> <pid> <dll_path>` 인자로 받아 같은 절차를 수행하고,
/// stdout에 대상 프로세스의 HMODULE을 출력한다.
pub fn inject_via_helper(helper: &Path, pid: u32, dll_path: &Path) -> Result<u64, InjectError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !helper.is_file() {
        return Err(InjectError::Helper(format!(
            "{} 파일이 없습니다",
            helper.display()
        )));
    }
    let output = std::process::Command::new(helper)
        .arg(pid.to_string())
        .arg(dll_path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| InjectError::Helper(error.to_string()))?;

    parse_inject_output(&output)
}

/// 헬퍼 프로세스의 종료 결과를 해석한다. 실제 프로세스 스폰과 분리해 둔
/// 순수한 갈림길이라, 테스트는 `ExitStatusExt::from_raw`로 만든 가짜
/// `Output`으로 stdout/stderr 파싱 분기를 결정적으로 검증할 수 있다.
fn parse_inject_output(output: &std::process::Output) -> Result<u64, InjectError> {
    if output.status.success() {
        let module = String::from_utf8_lossy(&output.stdout);
        return module.trim().parse::<u64>().map_err(|error| {
            InjectError::Helper(format!("모듈 핸들 출력이 잘못되었습니다: {error}"))
        });
    }
    // 헬퍼는 실패 원인을 stderr 한 줄로 남긴다.
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        return Err(InjectError::Helper(format!(
            "exit code {}",
            output.status.code().unwrap_or(-1)
        )));
    }
    Err(InjectError::Helper(detail.to_string()))
}

/// x86 게임용 헬퍼를 통해 방금 주입한 DLL을 제거한다.
pub fn uninject_via_helper(helper: &Path, pid: u32, module: u64) -> Result<(), InjectError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !helper.is_file() {
        return Err(InjectError::Helper(format!(
            "{} 파일이 없습니다",
            helper.display()
        )));
    }
    let module = u32::try_from(module)
        .map_err(|_| InjectError::Helper("x86 모듈 핸들이 범위를 벗어났습니다".to_string()))?;
    let output = std::process::Command::new(helper)
        .arg("--uninject")
        .arg(pid.to_string())
        .arg(module.to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| InjectError::Helper(error.to_string()))?;

    parse_uninject_output(&output)
}

/// [`parse_inject_output`]의 uninject 짝. 성공 시 값이 없다는 점만 다르다.
fn parse_uninject_output(output: &std::process::Output) -> Result<(), InjectError> {
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        return Err(InjectError::Helper(format!(
            "uninject exit code {}",
            output.status.code().unwrap_or(-1)
        )));
    }
    Err(InjectError::Helper(detail.to_string()))
}

#[cfg(test)]
#[path = "../../../tests/unit/hook/inject/helper.rs"]
mod tests;
