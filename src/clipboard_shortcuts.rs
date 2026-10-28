//! Observe Windows image-paste chords before egui-winit's text-only paste path.
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{WM_KEYDOWN, WM_KILLFOCUS, WM_NCDESTROY},
    },
};
const SUBCLASS: usize = 0x44554350;
struct State {
    alive: bool,
    paste: bool,
    ctx: crate::egui::Context,
}
pub(crate) struct Backend {
    hwnd: HWND,
    state: Box<State>,
}
fn paste_chord(
    message: u32,
    key: usize,
    repeat: bool,
    control: bool,
    shift: bool,
    alt: bool,
) -> bool {
    message == WM_KEYDOWN && key == 0x56 && !repeat && control && !shift && !alt
}
impl Backend {
    pub fn attach(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let handle = cc.window_handle().map_err(|error| error.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("No Windows window handle".into());
        };
        Self::install(handle.hwnd.get() as HWND, cc.egui_ctx.clone())
    }
    fn install(hwnd: HWND, ctx: crate::egui::Context) -> Result<Self, String> {
        let mut state = Box::new(State {
            alive: true,
            paste: false,
            ctx,
        });
        // SAFETY: UI-thread window; the boxed address remains stable until Drop
        // removes this subclass or NCDESTROY removes it before HWND reuse.
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(callback),
                SUBCLASS,
                (&mut *state as *mut State) as usize,
            )
        } == 0
        {
            return Err("Could not attach image paste shortcut".into());
        }
        Ok(Self { hwnd, state })
    }
    pub fn take_paste(&mut self) -> bool {
        std::mem::take(&mut self.state.paste)
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if self.state.alive {
            // SAFETY: same UI thread; state remains alive through removal.
            unsafe {
                RemoveWindowSubclass(self.hwnd, Some(callback), SUBCLASS);
            }
        }
    }
}
unsafe extern "system" fn callback(
    hwnd: HWND,
    message: u32,
    key: WPARAM,
    flags: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: data is the stable boxed State owned by Backend on the UI thread.
    // Window destruction removes this callback before its handle can be reused.
    unsafe {
        let state = &mut *(data as *mut State);
        if message == WM_NCDESTROY {
            state.alive = false;
            RemoveWindowSubclass(hwnd, Some(callback), SUBCLASS);
        } else if message == WM_KILLFOCUS {
            state.paste = false;
        } else if message == WM_KEYDOWN
            && paste_chord(
                message,
                key,
                flags & (1 << 30) != 0,
                GetKeyState(VK_CONTROL as i32) < 0,
                GetKeyState(VK_SHIFT as i32) < 0,
                GetKeyState(VK_MENU as i32) < 0,
            )
        {
            state.paste = true;
            state.ctx.request_repaint();
        }
        DefSubclassProc(hwnd, message, key, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW,
    };
    #[test]
    fn paste_chord_ignores_repeat_alt_shift_and_unrelated_messages() {
        assert!(paste_chord(WM_KEYDOWN, 0x56, false, true, false, false));
        for args in [
            (WM_KEYDOWN, 0x56, true, true, false, false),
            (WM_KEYDOWN, 0x56, false, false, false, false),
            (WM_KEYDOWN, 0x56, false, true, true, false),
            (WM_KEYDOWN, 0x56, false, true, false, true),
            (WM_KEYDOWN, 0x43, false, true, false, false),
            (WM_KILLFOCUS, 0x56, false, true, false, false),
        ] {
            assert!(!paste_chord(args.0, args.1, args.2, args.3, args.4, args.5));
        }
    }
    #[test]
    fn hidden_window_shortcut_detaches_safely_in_both_destruction_orders() {
        // SAFETY: hidden built-in STATIC windows and callbacks stay on this test
        // thread. Both HWND/box lifetime orders are exercised without clipboard IO.
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            for destroy_first in [false, true] {
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    1,
                    1,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!hwnd.is_null());
                let mut backend = Backend::install(hwnd, Default::default()).unwrap();
                backend.state.paste = true;
                assert!(backend.take_paste());
                assert!(!backend.take_paste());
                backend.state.paste = true;
                SendMessageW(hwnd, WM_KILLFOCUS, 0, 0);
                assert!(!backend.take_paste());
                if destroy_first {
                    assert_ne!(DestroyWindow(hwnd), 0);
                    assert!(!backend.state.alive);
                    drop(backend);
                } else {
                    drop(backend);
                    SendMessageW(hwnd, WM_KILLFOCUS, 0, 0);
                    assert_ne!(DestroyWindow(hwnd), 0);
                }
            }
        }
    }
}
