//! 긴 Unicode 경로를 지원하는 Vista+ Common Item Dialog helper.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, Result};

use crate::util::to_wide;

/// 파일 filter 한 항목. 여러 확장자는 `;`로 구분한다.
pub struct FileFilter<'a> {
    pub name: &'a str,
    pub spec: &'a str,
}

/// 필터 문자열들을 NUL-종료 와이드 버퍼로 변환하면서 수명을 유지하는 헬퍼.
struct FilterStorage {
    _wide: Vec<Vec<u16>>,
    specs: Vec<COMDLG_FILTERSPEC>,
}

fn build_filters(filters: &[FileFilter]) -> FilterStorage {
    let mut wide: Vec<Vec<u16>> = Vec::with_capacity(filters.len() * 2);
    let mut specs: Vec<COMDLG_FILTERSPEC> = Vec::with_capacity(filters.len());

    for f in filters {
        let name_w = to_wide(f.name);
        let spec_w = to_wide(f.spec);
        let name_ptr = PCWSTR(name_w.as_ptr());
        let spec_ptr = PCWSTR(spec_w.as_ptr());
        wide.push(name_w);
        wide.push(spec_w);
        specs.push(COMDLG_FILTERSPEC {
            pszName: name_ptr,
            pszSpec: spec_ptr,
        });
    }

    FilterStorage { _wide: wide, specs }
}

/// 유효한 `IShellItem`에서 파일 system 경로를 추출한다.
unsafe fn shell_item_to_path(item: &IShellItem) -> Result<PathBuf> {
    // SAFETY: 반환된 display-name pointer를 CoTaskMemFree로 해제한다.
    unsafe {
        let path_ptr = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = path_ptr.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(path_ptr.0 as *const _));
        Ok(path?)
    }
}

fn show_was_accepted(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Ok(false),
        Err(error) => Err(error),
    }
}

/// "열기" 다이얼로그 (단일 선택).
///
/// `title`/`filters`는 비어 있어도 된다. `Ok(None)`은 사용자 취소다.
pub fn open_file(hwnd: HWND, title: &str, filters: &[FileFilter]) -> Result<Option<PathBuf>> {
    // SAFETY: UI thread는 STA이며 COM interface는 이 scope 안에서만 쓴다.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)?;

        if !title.is_empty() {
            dialog.SetTitle(&HSTRING::from(title))?;
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            dialog.SetFileTypes(&storage.specs)?;
        }

        dialog.SetOptions(FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST)?;

        if !show_was_accepted(dialog.Show(Some(hwnd)))? {
            return Ok(None);
        }

        let item: IShellItem = dialog.GetResult()?;
        shell_item_to_path(&item).map(Some)
    }
}

/// 다중 선택 열기 dialog. `Ok(None)`은 사용자 취소다.
pub fn open_files_multi(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter],
) -> Result<Option<Vec<PathBuf>>> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)?;

        if !title.is_empty() {
            dialog.SetTitle(&HSTRING::from(title))?;
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            dialog.SetFileTypes(&storage.specs)?;
        }

        dialog.SetOptions(FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST | FOS_ALLOWMULTISELECT)?;

        if !show_was_accepted(dialog.Show(Some(hwnd)))? {
            return Ok(None);
        }

        let items = dialog.GetResults()?;

        let count = items.GetCount()?;
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            paths.push(shell_item_to_path(&items.GetItemAt(i)?)?);
        }
        Ok(Some(paths))
    }
}

/// 폴더 선택 다이얼로그.
pub fn pick_folder(hwnd: HWND, title: &str) -> Result<Option<PathBuf>> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)?;

        dialog.SetOptions(FOS_PICKFOLDERS)?;

        if !title.is_empty() {
            dialog.SetTitle(&HSTRING::from(title))?;
        }

        if !show_was_accepted(dialog.Show(Some(hwnd)))? {
            return Ok(None);
        }

        let item: IShellItem = dialog.GetResult()?;
        shell_item_to_path(&item).map(Some)
    }
}

/// 저장 dialog. `default_ext`는 점 없이 전달하며 `initial`은 초기 경로다.
pub fn save_file(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter],
    default_ext: Option<&str>,
    initial: Option<&std::path::Path>,
) -> Result<Option<PathBuf>> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_ALL)?;

        if !title.is_empty() {
            dialog.SetTitle(&HSTRING::from(title))?;
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            dialog.SetFileTypes(&storage.specs)?;
        }

        if let Some(ext) = default_ext {
            dialog.SetDefaultExtension(&HSTRING::from(ext))?;
        }

        if let Some(path) = initial {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                dialog.SetFileName(&HSTRING::from(name))?;
            }
            if let Some(folder) = path.parent().and_then(|s| s.to_str())
                && !folder.is_empty()
            {
                let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(folder), None)?;
                dialog.SetFolder(&item)?;
            }
        }

        dialog.SetOptions(FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT)?;

        if !show_was_accepted(dialog.Show(Some(hwnd)))? {
            return Ok(None);
        }

        let item: IShellItem = dialog.GetResult()?;
        shell_item_to_path(&item).map(Some)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/file_dialog.rs"]
mod tests;
