//! Showing the Windows touch keyboard. Windows offers it by itself only for its
//! own text controls, and Dudel's aren't, so the artist asks for it with a button.
use std::ffi::c_void;
use windows_sys::{
    Win32::{
        Foundation::HWND,
        System::Com::{COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx},
        UI::WindowsAndMessaging::GetDesktopWindow,
    },
    core::GUID,
};

/// The keyboard's host, and the undocumented interface that shows and hides it.
const HOST: GUID = GUID::from_u128(0x4ce576fa_83dc_4f88_951c_9d0782b4e376);
const INVOCATION: GUID = GUID::from_u128(0x37c994e7_432b_4834_a2f7_dce1f13b834b);
/// `CLSCTX_INPROC_HANDLER | CLSCTX_LOCAL_SERVER`.
const CONTEXT: u32 = 2 | 4;

#[repr(C)]
struct Vtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    toggle: unsafe extern "system" fn(*mut c_void, HWND) -> i32,
}

/// Ask the running host to show or hide the keyboard. Its HRESULT on failure.
fn toggle_running() -> Result<(), i32> {
    let mut object: *mut c_void = std::ptr::null_mut();
    // SAFETY: `object` receives a COM pointer to the interface named by
    // INVOCATION, whose first field is the vtable declared above. It is used
    // only when creation succeeds, and released once.
    unsafe {
        // Already initialized on the UI thread, which this may report; both
        // are fine for the call below.
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let created = CoCreateInstance(
            &HOST,
            std::ptr::null_mut(),
            CONTEXT,
            &INVOCATION,
            &mut object,
        );
        if created < 0 {
            return Err(created);
        }
        let vtable = *(object as *mut *const Vtable);
        let toggled = ((*vtable).toggle)(object, GetDesktopWindow());
        ((*vtable).release)(object);
        if toggled < 0 {
            return Err(toggled);
        }
    }
    Ok(())
}

/// Show the touch keyboard, or hide it if it is showing.
pub(crate) fn toggle() -> Result<(), String> {
    if toggle_running().is_ok() {
        return Ok(());
    }
    // The host starts on demand, and starting it shows the keyboard.
    let common = std::env::var_os("CommonProgramW6432")
        .or_else(|| std::env::var_os("CommonProgramFiles"))
        .ok_or("Could not find the touch keyboard")?;
    let path = std::path::Path::new(&common).join(r"microsoft shared\ink\TabTip.exe");
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open the touch keyboard: {error}"))
}

#[cfg(test)]
mod tests {
    /// Shows and hides the keyboard on the real desktop:
    /// `cargo test touch_keyboard -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn touch_keyboard_round_trip() {
        println!("first: {:?}", super::toggle());
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("second: {:?}", super::toggle_running());
    }
}
