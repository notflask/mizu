//! Windows: which end of the pen touches. Pressure already arrives with
//! winit's `Touch` events; the eraser end does not, so the window is
//! subclassed to look at `WM_POINTER*` messages before winit sees them.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, POINTER_PEN_INFO};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_ERASER, PEN_FLAG_INVERTED, WM_POINTERDOWN, WM_POINTERUPDATE,
};
use winit::window::Window;

use super::Emit;

/// Whether the pen that last touched used its eraser end.
static ERASER: AtomicBool = AtomicBool::new(false);

const SUBCLASS_ID: usize = 0x6d697a75; // "mizu"

pub fn eraser_active() -> bool {
    ERASER.load(Ordering::Relaxed)
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_POINTERDOWN || msg == WM_POINTERUPDATE {
        let pointer_id = (wparam.0 & 0xffff) as u32;
        let mut info = POINTER_PEN_INFO::default();
        // SAFETY: valid out pointer; fails harmlessly for non-pen pointers.
        if unsafe { GetPointerPenInfo(pointer_id, &mut info) }.is_ok() {
            let eraser = info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0;
            ERASER.store(eraser, Ordering::Relaxed);
        }
    }
    // SAFETY: forwarding the unchanged message down the subclass chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

struct Subclass(HWND);

impl Drop for Subclass {
    fn drop(&mut self) {
        // SAFETY: removes what `init` installed on this window.
        unsafe {
            let _ = RemoveWindowSubclass(self.0, Some(subclass_proc), SUBCLASS_ID);
        }
    }
}

pub fn init(window: &Window, _emit: Emit) -> Option<Box<dyn Any>> {
    let RawWindowHandle::Win32(h) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    // SAFETY: the window belongs to this thread and outlives the subclass
    // (it is removed in Drop).
    let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        log::warn!("windows: cannot subclass the window for pen input");
        return None;
    }
    Some(Box::new(Subclass(hwnd)))
}
