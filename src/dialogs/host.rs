//! Resource dialog의 공통 생성, 상태 등록, DPI 재진입과 파괴 경계.

use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::{E_FAIL, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::Input::KeyboardAndMouse::EnableWindow,
        UI::WindowsAndMessaging::{
            CreateDialogParamW, DWL_USER, DestroyWindow, GetWindowLongPtrW, GetWindowRect,
            IsWindow, MSG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow,
            SetWindowLongPtrW, SetWindowPos, WINDOW_LONG_PTR_INDEX, WM_CLOSE, WM_DESTROY,
            WM_DPICHANGED,
        },
    },
    core::{Error, PCWSTR, Result},
};

use super::helpers::{
    center_dialog_on_monitor, defer_dialog_dpi_change, defer_dialog_message,
    flush_deferred_dialog_messages, register_resource_dialog, rescale_dialog_children_for_dpi,
    show_dialog_window, unregister_resource_dialog,
};

const DWLP_USER_INDEX: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(DWL_USER as i32);

unsafe fn set_dialog_user(hwnd: HWND, value: isize) {
    unsafe { SetWindowLongPtrW(hwnd, DWLP_USER_INDEX, value) };
}

thread_local! {
    static HOSTED_DIALOGS: RefCell<HashMap<TypeId, HWND>> = RefCell::new(HashMap::new());
    /// `DISABLE_PARENT` dialog가 파괴될 때 다시 활성화할 부모 창.
    static DISABLED_PARENTS: RefCell<HashMap<TypeId, HWND>> = RefCell::new(HashMap::new());
}

pub(crate) enum DialogResult {
    Handled(LRESULT),
    Close(LRESULT),
    Unhandled,
}

/// 새로 만든 창을 처음 놓을 위치.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogPlacement {
    /// 부모가 있는 monitor의 작업 영역 중앙.
    MonitorCenter,
    /// 부모 창의 사각형 중앙. 부모를 가리는 modal 스타일 창에 쓴다.
    ParentCenter,
}

/// 같은 dialog를 다시 열려고 할 때의 동작.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReopenPolicy {
    /// 기존 창을 앞으로 가져오고 그 HWND를 반환한다.
    Activate,
    /// 기존 창을 앞으로 가져오되 오류를 반환해 호출자가 중복 실행을 막게 한다.
    Reject(&'static str),
}

pub(crate) trait HostedDialog: Sized + 'static {
    type Init: 'static;
    const RESOURCE_ID: u16;

    /// 창을 띄우기 전 초기 배치 방식.
    const PLACEMENT: DialogPlacement = DialogPlacement::MonitorCenter;

    /// 창이 살아 있는 동안 부모 입력을 막을지. `true`면 `WM_DESTROY`에서 다시 활성화한다.
    const DISABLE_PARENT: bool = false;

    /// 이미 열려 있을 때의 처리.
    const REOPEN: ReopenPolicy = ReopenPolicy::Activate;

    fn create(hwnd: HWND, init: Self::Init) -> Result<Self>;

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> DialogResult;

    /// DPI 변경 시 자식을 다시 배치하기 위한 현재 적용 DPI. `None`이면 host가
    /// 기본 rescale을 건너뛴다.
    fn applied_dpi(&mut self) -> Option<&mut u32> {
        None
    }

    /// 기본 rescale과 창 이동이 끝난 뒤 호출된다. 탭 높이 재계산처럼 dialog별
    /// 후처리가 필요할 때만 구현한다.
    fn after_dpi_changed(&mut self, _new_dpi: u32) {}

    fn destroy(&mut self) {}

    /// 재진입 시 LPARAM/WPARAM을 queue에 복사해도 수명이 안전한 message만 허용한다.
    fn can_defer(_msg: u32) -> bool {
        false
    }

    /// state를 빌리기 전에 처리해야 하는 message. `WM_CTLCOLORSTATIC`처럼
    /// 재진입 중에도 반드시 응답해야 하는 경우에 쓴다.
    fn handle_before_borrow(
        _hwnd: HWND,
        _msg: u32,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Option<isize> {
        None
    }

    /// `IsDialogMessageW`가 Enter/Tab 등을 기본 dialog 동작으로 바꾸기 전에
    /// dialog별로 가로챌 메시지를 처리한다.
    unsafe fn pretranslate_message(_hwnd: HWND, _msg: &MSG) -> bool {
        false
    }
}

/// `WM_DPICHANGED` 공통 처리: 자식 rescale 후 OS가 제안한 사각형으로 창을 옮긴다.
fn apply_dpi_change<T: HostedDialog>(dialog: &mut T, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let new_dpi = (wparam.0 & 0xffff) as u32;
    if let Some(applied) = dialog.applied_dpi() {
        rescale_dialog_children_for_dpi(hwnd, *applied, new_dpi);
        *applied = new_dpi;
    }

    if lparam.0 != 0 {
        // SAFETY: WM_DPICHANGED의 LPARAM은 메시지 처리 동안 유효한 RECT 포인터다.
        unsafe {
            let rect = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    dialog.after_dpi_changed(new_dpi);
}

/// 부모 창 사각형의 중앙에 dialog를 놓는다.
///
/// # Safety
/// `hwnd`와 `parent`는 유효한 window handle이어야 한다.
unsafe fn center_dialog_on_parent(hwnd: HWND, parent: HWND) {
    unsafe {
        let mut dialog_rect = RECT::default();
        let mut parent_rect = RECT::default();
        if GetWindowRect(hwnd, &mut dialog_rect).is_err()
            || GetWindowRect(parent, &mut parent_rect).is_err()
        {
            return;
        }
        let width = dialog_rect.right - dialog_rect.left;
        let height = dialog_rect.bottom - dialog_rect.top;
        let x = parent_rect.left + (parent_rect.right - parent_rect.left - width) / 2;
        let y = parent_rect.top + (parent_rect.bottom - parent_rect.top - height) / 2;
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
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
            return match T::REOPEN {
                ReopenPolicy::Activate => Ok(hwnd),
                ReopenPolicy::Reject(message) => Err(Error::new(E_FAIL, message)),
            };
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
            match T::PLACEMENT {
                DialogPlacement::MonitorCenter => center_dialog_on_monitor(hwnd, parent),
                DialogPlacement::ParentCenter => center_dialog_on_parent(hwnd, parent),
            }
            if T::DISABLE_PARENT {
                let _ = EnableWindow(parent, false);
                DISABLED_PARENTS.with(|parents| {
                    parents.borrow_mut().insert(TypeId::of::<T>(), parent);
                });
            }
            show_dialog_window(hwnd);
        }
        Ok(hwnd)
    }

    /// 열려 있는 dialog의 state를 빌려 읽는다. 창이 없거나 이미 대여 중이면 `None`.
    pub(crate) fn with_state<R>(f: impl FnOnce(&T) -> R) -> Option<R> {
        let hwnd = Self::current_hwnd()?;
        // SAFETY: 등록된 창의 DWLP_USER에는 WM_DESTROY 전까지 유효한 state가 있다.
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, DWLP_USER_INDEX) } as *mut RefCell<T>;
        if state_ptr.is_null() {
            return None;
        }
        // SAFETY: state_ptr는 같은 UI thread가 소유하며 WM_DESTROY까지 살아 있다.
        let state = unsafe { &*state_ptr };
        let dialog = state.try_borrow().ok()?;
        Some(f(&dialog))
    }

    /// 열려 있는 dialog의 state를 가변으로 빌린다. 재진입 중이면 `None`.
    pub(crate) fn with_state_mut<R>(f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let hwnd = Self::current_hwnd()?;
        // SAFETY: 등록된 창의 DWLP_USER에는 WM_DESTROY 전까지 유효한 state가 있다.
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, DWLP_USER_INDEX) } as *mut RefCell<T>;
        if state_ptr.is_null() {
            return None;
        }
        // SAFETY: state_ptr는 같은 UI thread가 소유하며 WM_DESTROY까지 살아 있다.
        let state = unsafe { &*state_ptr };
        let mut dialog = state.try_borrow_mut().ok()?;
        Some(f(&mut dialog))
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
        if let Some(result) = T::handle_before_borrow(hwnd, msg, wparam, lparam) {
            return result;
        }

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
                    register_resource_dialog(hwnd, T::pretranslate_message);
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
            drop(state);
            // 부모를 다시 활성화하는 것은 state 정리 뒤에 한다. 그래야 포커스가
            // 돌아간 부모가 즉시 보내는 message가 이미 회수된 state를 건드리지 않는다.
            if T::DISABLE_PARENT {
                let parent = DISABLED_PARENTS
                    .with(|parents| parents.borrow_mut().remove(&TypeId::of::<T>()));
                if let Some(parent) = parent {
                    // SAFETY: 부모는 show에서 저장한 같은 UI thread의 창이다.
                    unsafe {
                        let _ = EnableWindow(parent, true);
                        let _ = SetForegroundWindow(parent);
                    }
                }
            }
            return 1;
        }

        // SAFETY: state 포인터는 WM_DESTROY 전까지 유효하다.
        let state = unsafe { &*state_ptr };
        if msg == WM_DPICHANGED {
            match state.try_borrow_mut() {
                Ok(mut dialog) => {
                    apply_dpi_change(&mut *dialog, hwnd, wparam, lparam);
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
                // WM_CLOSE는 pointer-free이며, 바깥 handler의 대여가 끝난 뒤 다시
                // 처리해야 한다. 여기서 DestroyWindow를 호출하면 동기 WM_DESTROY가
                // 아직 살아 있는 RefMut 아래의 state를 해제한다.
                if msg == WM_CLOSE {
                    unsafe { defer_dialog_message(hwnd, msg, wparam, lparam) };
                    return 1;
                }
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
            // WM_CLOSE를 직접 처리하지 않는 dialog는 기본 동작대로 창을 닫는다.
            // 진행률 창처럼 닫기를 취소로 바꿔야 하는 쪽은 Handled를 돌려 이 경로를 피한다.
            DialogResult::Unhandled if msg == WM_CLOSE => {
                // SAFETY: dialog borrow는 match 전에 해제됐다.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                1
            }
            DialogResult::Unhandled => 0,
        }
    }
}
