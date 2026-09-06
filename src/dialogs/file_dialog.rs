//! Raw Win32 file dialog helpers.

use crate::win32::to_wide;
use std::path::{Path, PathBuf};
use windows_core::{Error, HRESULT};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER,
    OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, GPFIDL_DEFAULT, SHBrowseForFolderW,
    SHGetPathFromIDListEx,
};

pub struct FileFilter<'a> {
    pub name: &'a str,
    pub spec: &'a str,
}

fn error(message: &str) -> Error {
    Error::new(HRESULT(0x80004005u32 as i32), message)
}

/// Common-dialog APIs return zero for both cancellation and failure. The
/// extended error code is zero only for user cancellation, so retain it in the
/// returned HRESULT for callers that need to distinguish the two cases.
fn common_dialog_error(api: &str, code: u32) -> Error {
    let hresult = HRESULT((0x8007_0000u32 | code) as i32);
    Error::new(
        hresult,
        format!("{api} failed (CommDlgExtendedError={code})"),
    )
}

/// `GetOpenFileNameW`/`GetSaveFileNameW` return zero for both cancel and error.
/// A zero extended code is the documented cancellation case.
fn classify_common_dialog_result(api: &str, extended_error: u32) -> windows_core::Result<()> {
    if extended_error == 0 {
        Ok(())
    } else {
        Err(common_dialog_error(api, extended_error))
    }
}

fn filter_wide(filters: &[FileFilter<'_>]) -> Vec<u16> {
    let mut out = Vec::new();
    for filter in filters {
        out.extend(filter.name.encode_utf16());
        out.push(0);
        out.extend(filter.spec.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

fn parse_selection(buffer: &[u16], multi: bool) -> Vec<PathBuf> {
    let mut fields = Vec::new();
    let mut start = 0;
    for (index, value) in buffer.iter().enumerate() {
        if *value == 0 {
            if index == start {
                break;
            }
            fields.push(String::from_utf16_lossy(&buffer[start..index]));
            start = index + 1;
        }
    }
    if fields.is_empty() {
        return Vec::new();
    }
    if !multi || fields.len() == 1 {
        return vec![PathBuf::from(&fields[0])];
    }
    let dir = PathBuf::from(&fields[0]);
    fields[1..].iter().map(|name| dir.join(name)).collect()
}

fn open_raw(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter<'_>],
    multi: bool,
) -> windows_core::Result<Option<Vec<PathBuf>>> {
    let mut file = vec![0u16; 32 * 1024];
    let title_w = to_wide(title);
    let filter = filter_wide(filters);
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title_w.as_ptr(),
        Flags: OFN_EXPLORER
            | OFN_PATHMUSTEXIST
            | OFN_FILEMUSTEXIST
            | if multi { OFN_ALLOWMULTISELECT } else { 0 },
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        let code = unsafe { CommDlgExtendedError() };
        classify_common_dialog_result("GetOpenFileNameW", code)?;
        return Ok(None);
    }
    Ok(Some(parse_selection(&file, multi)))
}

pub fn open_file(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter<'_>],
) -> windows_core::Result<Option<PathBuf>> {
    Ok(open_raw(hwnd, title, filters, false)?.and_then(|mut paths| paths.pop()))
}
pub fn open_files_multi(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter<'_>],
) -> windows_core::Result<Option<Vec<PathBuf>>> {
    open_raw(hwnd, title, filters, true)
}
pub fn pick_folder(hwnd: HWND, title: &str) -> windows_core::Result<Option<PathBuf>> {
    let title_w = to_wide(title);
    // BROWSEINFO's display buffer is only a visual hint; SHGetPathFromIDListEx
    // writes the selected filesystem path into the separate output buffer.
    let mut display_name = vec![0u16; 260];
    let mut path = vec![0u16; 32 * 1024];
    let browse = BROWSEINFOW {
        hwndOwner: hwnd,
        pszDisplayName: display_name.as_mut_ptr(),
        lpszTitle: title_w.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };

    // SHBrowseForFolderW returns null when the user cancels. The PIDL is
    // allocated by the shell task allocator and must be released regardless of
    // whether converting it to a filesystem path succeeds.
    let pidl = unsafe { SHBrowseForFolderW(&browse) };
    if pidl.is_null() {
        return Ok(None);
    }
    let converted = unsafe {
        SHGetPathFromIDListEx(pidl, path.as_mut_ptr(), path.len() as u32, GPFIDL_DEFAULT)
    } != 0;
    unsafe { CoTaskMemFree(pidl.cast()) };
    if !converted {
        return Err(error("선택한 폴더의 경로를 읽지 못했습니다"));
    }

    let length = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    if length == 0 {
        return Err(error("선택한 폴더의 경로가 비어 있습니다"));
    }
    Ok(Some(PathBuf::from(String::from_utf16_lossy(
        &path[..length],
    ))))
}
pub fn save_file(
    hwnd: HWND,
    title: &str,
    filters: &[FileFilter<'_>],
    default_ext: Option<&str>,
    initial: Option<&Path>,
) -> windows_core::Result<Option<PathBuf>> {
    let mut file = vec![0u16; 32 * 1024];
    if let Some(path) = initial {
        let wide = to_wide(&path.to_string_lossy());
        let n = wide.len().min(file.len());
        file[..n].copy_from_slice(&wide[..n]);
    }
    let title_w = to_wide(title);
    let filter = filter_wide(filters);
    let ext = default_ext.map(to_wide);
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title_w.as_ptr(),
        lpstrDefExt: ext.as_ref().map_or(std::ptr::null(), std::vec::Vec::as_ptr),
        Flags: OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT,
        ..Default::default()
    };
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        let code = unsafe { CommDlgExtendedError() };
        classify_common_dialog_result("GetSaveFileNameW", code)?;
        return Ok(None);
    }
    Ok(parse_selection(&file, false).pop())
}

#[cfg(test)]
#[path = "../../tests/unit/dialogs/file_dialog.rs"]
mod tests;
