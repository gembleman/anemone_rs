//! 모던 파일 다이얼로그 헬퍼 (Common Item Dialog, Vista+)
//!
//! `IFileOpenDialog` / `IFileSaveDialog` 기반. 레거시 `GetOpenFileNameW`/
//! `GetSaveFileNameW` 를 대체한다. Win10 셸 모양 그대로 표시되고
//! MAX_PATH 제약 없이 긴 경로/유니코드를 지원한다.

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::PCWSTR;

use crate::util::to_wide;

/// 파일 필터 한 항목.
///
/// `name` 예: "텍스트 파일", `spec` 예: "*.txt".
/// 여러 확장자는 ";" 로 구분한다 ("*.txt;*.log").
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

/// IShellItem 에서 파일 시스템 경로를 추출한다.
///
/// SAFETY: 호출자는 `item` 이 유효한 COM 인터페이스임을 보장해야 한다.
unsafe fn shell_item_to_path(item: &IShellItem) -> Option<PathBuf> {
    // SAFETY: GetDisplayName 은 유효한 COM 호출이며, 반환된 포인터는
    // CoTaskMemFree 로 해제한다.
    unsafe {
        let path_ptr = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .inspect_err(|e| tracing::warn!("IShellItem::GetDisplayName failed: {e}"))
            .ok()?;
        let path = path_ptr
            .to_string()
            .inspect_err(|e| tracing::warn!("Path conversion failed: {e}"))
            .ok()
            .map(PathBuf::from);
        CoTaskMemFree(Some(path_ptr.0 as *const _));
        path
    }
}

/// "열기" 다이얼로그 (단일 선택).
///
/// `title`/`filters` 는 비어 있어도 된다. 반환 `None` 은 취소 또는 실패.
pub fn open_file(hwnd: HWND, title: &str, filters: &[FileFilter]) -> Option<PathBuf> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다. 모든 COM
    // 인터페이스는 이 함수 안에서만 사용되고 Drop 시 자동 Release 된다.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)
            .inspect_err(|e| tracing::warn!("CoCreateInstance(FileOpenDialog) failed: {e}"))
            .ok()?;

        if !title.is_empty() {
            let title_w = to_wide(title);
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            let _ = dialog.SetFileTypes(&storage.specs);
        }

        let _ = dialog.SetOptions(FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST);

        if dialog.Show(Some(hwnd)).is_err() {
            return None;
        }

        let item: IShellItem = dialog.GetResult().ok()?;
        shell_item_to_path(&item)
    }
}

/// "열기" 다이얼로그 (다중 선택).
///
/// 취소 시 빈 벡터를 반환한다.
pub fn open_files_multi(hwnd: HWND, title: &str, filters: &[FileFilter]) -> Vec<PathBuf> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileOpenDialog = match CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("CoCreateInstance(FileOpenDialog) failed: {e}");
                return Vec::new();
            }
        };

        if !title.is_empty() {
            let title_w = to_wide(title);
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            let _ = dialog.SetFileTypes(&storage.specs);
        }

        let _ = dialog.SetOptions(
            FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST | FOS_ALLOWMULTISELECT,
        );

        if dialog.Show(Some(hwnd)).is_err() {
            return Vec::new();
        }

        let items = match dialog.GetResults() {
            Ok(a) => a,
            Err(_) => return Vec::new(),
        };

        let count = items.GetCount().unwrap_or(0);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            if let Ok(item) = items.GetItemAt(i) {
                if let Some(p) = shell_item_to_path(&item) {
                    paths.push(p);
                }
            }
        }
        paths
    }
}

/// 폴더 선택 다이얼로그.
pub fn pick_folder(hwnd: HWND, title: &str) -> Option<PathBuf> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL)
            .inspect_err(|e| tracing::warn!("CoCreateInstance(FileOpenDialog) failed: {e}"))
            .ok()?;

        let _ = dialog.SetOptions(FOS_PICKFOLDERS);

        if !title.is_empty() {
            let title_w = to_wide(title);
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
        }

        if dialog.Show(Some(hwnd)).is_err() {
            return None;
        }

        let item: IShellItem = dialog.GetResult().ok()?;
        shell_item_to_path(&item)
    }
}

/// "저장" 다이얼로그.
///
/// `default_ext` 는 점 없이 ("txt"). `initial` 이 주어지면 그 경로의
/// 폴더와 파일명을 기본값으로 채운다.
pub fn save_file(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter],
    default_ext: Option<&str>,
    initial: Option<&std::path::Path>,
) -> Option<PathBuf> {
    // SAFETY: UI 스레드는 main()에서 STA 로 1회 초기화되어 있다.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_ALL)
            .inspect_err(|e| tracing::warn!("CoCreateInstance(FileSaveDialog) failed: {e}"))
            .ok()?;

        if !title.is_empty() {
            let title_w = to_wide(title);
            let _ = dialog.SetTitle(PCWSTR(title_w.as_ptr()));
        }

        let storage = build_filters(filters);
        if !storage.specs.is_empty() {
            let _ = dialog.SetFileTypes(&storage.specs);
        }

        if let Some(ext) = default_ext {
            let ext_w = to_wide(ext);
            let _ = dialog.SetDefaultExtension(PCWSTR(ext_w.as_ptr()));
        }

        if let Some(path) = initial {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                let name_w = to_wide(name);
                let _ = dialog.SetFileName(PCWSTR(name_w.as_ptr()));
            }
            if let Some(folder) = path.parent().and_then(|s| s.to_str()) {
                if !folder.is_empty() {
                    let folder_w = to_wide(folder);
                    let item: windows::core::Result<IShellItem> =
                        SHCreateItemFromParsingName(PCWSTR(folder_w.as_ptr()), None);
                    if let Ok(item) = item {
                        let _ = dialog.SetFolder(&item);
                    }
                }
            }
        }

        let _ = dialog.SetOptions(FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT);

        if dialog.Show(Some(hwnd)).is_err() {
            return None;
        }

        let item: IShellItem = dialog.GetResult().ok()?;
        shell_item_to_path(&item)
    }
}
