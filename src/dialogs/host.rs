//! Resource dialog의 공통 생성, 상태 등록, DPI 재진입과 파괴 경계.

use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{E_FAIL, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateDialogParamW, DWL_USER, DestroyWindow, GetWindowLongPtrW, IsWindow,
            SetForegroundWindow, SetWindowLongPtrW, WINDOW_LONG_PTR_INDEX, WM_CLOSE, WM_DESTROY,
            WM_DPICHANGED,
        },
    },
    core::{Error, PCWSTR, Result},
};

use super::helpers::{
    center_dialog_on_monitor, defer_dialog_dpi_change, defer_dialog_message,
    flush_deferred_dialog_messages, register_resource_dialog, show_dialog_window,
    unregister_resource_dialog,
};

const DWLP_USER_INDEX: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(DWL_USER as i32);

#[cfg(target_pointer_width = "64")]
unsafe fn set_dialog_user(hwnd: HWND, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, DWLP_USER_INDEX, value) };
}

#[cfg(target_pointer_width = "32")]
unsafe fn set_dialog_user(hwnd: HWND, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, DWLP_USER_INDEX, value as i32) };
}

thread_local! {
    static HOSTED_DIALOGS: RefCell<HashMap<TypeId, HWND>> = RefCell::new(HashMap::new());
}

pub(crate) enum DialogResult {
    Handled(LRESULT),
    Close(LRESULT),
    Unhandled,
}

pub(crate) trait HostedDialog: Sized + 'static {
    type Init: 'static;
    const RESOURCE_ID: u16;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self>;

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> DialogResult;

    fn handle_dpi_changed(&mut self, _wparam: WPARAM, _lparam: LPARAM) {}

    fn destroy(&mut self) {}

    /// 재진입 시 LPARAM/WPARAM을 queue에 복사해도 수명이 안전한 message만 허용한다.
    fn can_defer(_msg: u32) -> bool {
        false
    }
}

struct InitEnvelope<T: HostedDialog> {
    init: Option<T::Init>,
    error: Rc<RefCell<Option<String>>>,
    consumed: Rc<Cell<bool>>,
}

pub(crate) struct DialogHost<T>(PhantomData<T>);

impl<T: HostedDialog> DialogHost<T> {
    pub(crate) fn show(parent: HWND, init: T::Init) -> Result<HWND> {
        if let Some(hwnd) = Self::current_hwnd() {
            // SAFETY: registry에는 IsWindow로 검증된 같은 UI thread의 dialog만 남긴다.
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(hwnd);
        }

        let error = Rc::new(RefCell::new(None));
        let consumed = Rc::new(Cell::new(false));
        let envelope = Box::new(InitEnvelope::<T> {
            init: Some(init),
            error: error.clone(),
            consumed: consumed.clone(),
        });
        let raw = Box::into_raw(envelope);
        // SAFETY: raw envelope는 WM_INITDIALOG에서 정확히 한 번 회수한다. 콜백 전에
        // 생성이 실패하면 아래 consumed 검사에서 호출자가 회수한다.
        let created = unsafe {
            let instance = GetModuleHandleW(None)?;
            CreateDialogParamW(
                Some(instance.into()),
                PCWSTR(T::RESOURCE_ID as usize as *const u16),
                Some(parent),
                Some(Self::dialog_proc),
                LPARAM(raw as isize),
            )
        };

        let hwnd = match created {
            Ok(hwnd) => hwnd,
            Err(create_error) => {
                if !consumed.get() {
                    // SAFETY: WM_INITDIALOG가 envelope를 소비하지 않았다.
                    unsafe { drop(Box::from_raw(raw)) };
                }
                return Err(create_error);
            }
        };

        if let Some(message) = error.borrow_mut().take() {
            // SAFETY: CreateDialogParamW가 반환한 유효한 모델리스 dialog.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(Error::new(E_FAIL, message));
        }

        // SAFETY: hwnd와 parent는 같은 UI thread의 유효한 window handle이다.
        unsafe {
            center_dialog_on_monitor(hwnd, parent);
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    pub(crate) fn current_hwnd() -> Option<HWND> {
        let hwnd =
            HOSTED_DIALOGS.with(|dialogs| dialogs.borrow().get(&TypeId::of::<T>()).copied())?;
        // SAFETY: HWND 값 자체는 복사 가능하며 유효성만 조회한다.
        unsafe { IsWindow(Some(hwnd)).as_bool().then_some(hwnd) }
    }

    unsafe extern "system" fn dialog_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> isize {
        if msg == windows::Win32::UI::WindowsAndMessaging::WM_INITDIALOG {
            if lparam.0 == 0 {
                return 0;
            }
            // SAFETY: lparam은 show가 Box::into_raw로 전달한 정확한 envelope 포인터다.
            let mut envelope = unsafe { Box::from_raw(lparam.0 as *mut InitEnvelope<T>) };
            envelope.consumed.set(true);
            let init = envelope
                .init
                .take()
                .expect("hosted dialog init is consumed once");
            match T::create(hwnd, init) {
                Ok(dialog) => {
                    let state = Box::into_raw(Box::new(RefCell::new(dialog)));
                    // SAFETY: DWLP_USER는 dialog가 파괴될 때까지 state 포인터를 보존한다.
                    unsafe { set_dialog_user(hwnd, state as isize) };
                    HOSTED_DIALOGS.with(|dialogs| {
                        dialogs.borrow_mut().insert(TypeId::of::<T>(), hwnd);
                    });
                    register_resource_dialog(hwnd);
                    return 1;
                }
                Err(create_error) => {
                    *envelope.error.borrow_mut() = Some(create_error.to_string());
                    return 0;
                }
            }
        }

        // SAFETY: host가 초기화에 성공한 dialog만 DWLP_USER에 state를 저장한다.
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, DWLP_USER_INDEX) } as *mut RefCell<T>;
        if state_ptr.is_null() {
            return 0;
        }

        if msg == WM_DESTROY {
            // SAFETY: 파괴는 host가 mutable borrow를 해제한 뒤 실행하므로 state를 회수할 수 있다.
            unsafe { set_dialog_user(hwnd, 0) };
            HOSTED_DIALOGS.with(|dialogs| {
                dialogs.borrow_mut().remove(&TypeId::of::<T>());
            });
            unregister_resource_dialog(hwnd);
            // SAFETY: state_ptr는 WM_INITDIALOG에서 Box::into_raw로 만든 유일한 포인터다.
            let state = unsafe { Box::from_raw(state_ptr) };
            if let Ok(mut dialog) = state.try_borrow_mut() {
                dialog.destroy();
            }
            return 1;
        }

        if msg == WM_CLOSE {
            // SAFETY: state borrow 전에 처리하며 DestroyWindow 재진입 시 host가 정리한다.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return 1;
        }

        // SAFETY: state 포인터는 WM_DESTROY 전까지 유효하다.
        let state = unsafe { &*state_ptr };
        if msg == WM_DPICHANGED {
            match state.try_borrow_mut() {
                Ok(mut dialog) => {
                    dialog.handle_dpi_changed(wparam, lparam);
                    drop(dialog);
                    flush_deferred_dialog_messages(hwnd);
                }
                Err(_) => unsafe { defer_dialog_dpi_change(hwnd, wparam, lparam) },
            }
            return 1;
        }

        let response = match state.try_borrow_mut() {
            Ok(mut dialog) => dialog.handle_message(msg, wparam, lparam),
            Err(_) => {
                if T::can_defer(msg) {
                    unsafe { defer_dialog_message(hwnd, msg, wparam, lparam) };
                    return 1;
                }
                return 0;
            }
        };
        flush_deferred_dialog_messages(hwnd);
        match response {
            DialogResult::Handled(result) => result.0,
            DialogResult::Close(result) => {
                // SAFETY: dialog borrow는 match 전에 해제됐다.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                result.0
            }
            DialogResult::Unhandled => 0,
        }
    }
}
