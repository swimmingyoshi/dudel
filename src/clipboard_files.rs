//! Bounded CF_HDROP decoding. Do not pass external clipboard bytes to
//! DragQueryFileW: malformed file lists can fault inside Windows.Storage.dll.
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use windows_sys::Win32::{
    Globalization::{CP_ACP, MultiByteToWideChar},
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    },
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_PATH: usize = 32767;
const CF_HDROP: u32 = 15;

pub(crate) fn read() -> Vec<PathBuf> {
    // SAFETY: read-only clipboard ownership and global-memory lock are scoped
    // to this thread. Never free the memory, which belongs to the clipboard.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Vec::new();
        }
        struct Clipboard;
        impl Drop for Clipboard {
            fn drop(&mut self) {
                unsafe { CloseClipboard() };
            }
        }
        let _clipboard = Clipboard;
        let handle = GetClipboardData(CF_HDROP);
        if handle.is_null() {
            return Vec::new();
        }
        let size = GlobalSize(handle);
        if !(22..=MAX_BYTES).contains(&size) {
            return Vec::new();
        }
        let data = GlobalLock(handle);
        if data.is_null() {
            return Vec::new();
        }
        struct Lock(windows_sys::Win32::Foundation::HGLOBAL);
        impl Drop for Lock {
            fn drop(&mut self) {
                unsafe { GlobalUnlock(self.0) };
            }
        }
        let _lock = Lock(handle);
        // GlobalSize bounds this locked allocation; the clipboard cannot be
        // replaced while open. Parsing never reads beyond this slice.
        parse(std::slice::from_raw_parts(data.cast(), size)).unwrap_or_default()
    }
}

fn parse(data: &[u8]) -> Option<Vec<PathBuf>> {
    if !(22..=MAX_BYTES).contains(&data.len()) {
        return None;
    }
    let offset = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let wide = u32::from_le_bytes(data[16..20].try_into().ok()?);
    if offset < 20 || offset >= data.len() {
        return None;
    }
    let payload = &data[offset..];
    let names = if wide != 0 {
        if !payload.len().is_multiple_of(2) {
            return None;
        }
        split_names(
            &payload
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )?
    } else {
        split_names(payload)?
            .into_iter()
            .map(|bytes| ansi(&bytes))
            .collect::<Option<Vec<_>>>()?
    };
    Some(
        names
            .into_iter()
            .map(|name| PathBuf::from(OsString::from_wide(&name)))
            .collect(),
    )
}

/// Require a complete double-terminated list; reject the entire list on damage.
fn split_names<T: Copy + Default + PartialEq>(data: &[T]) -> Option<Vec<Vec<T>>> {
    let mut names = Vec::new();
    let mut start = 0;
    loop {
        let end = start + data.get(start..)?.iter().position(|c| *c == T::default())?;
        if start == end {
            if start == 0 && data.get(1) != Some(&T::default()) {
                return None;
            }
            return Some(names);
        }
        if end - start > MAX_PATH || names.len() == MAX_FILES {
            return None;
        }
        names.push(data[start..end].to_vec());
        start = end + 1;
    }
}

fn ansi(bytes: &[u8]) -> Option<Vec<u16>> {
    // SAFETY: lengths are bounded by MAX_PATH and both buffers cover the exact
    // lengths supplied. Use the system ANSI code page required by DROPFILES.
    unsafe {
        let size = MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if size <= 0 {
            return None;
        }
        let mut result = vec![0; size as usize];
        (MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            result.as_mut_ptr(),
            size,
        ) == size)
            .then_some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(wide: bool) -> Vec<u8> {
        let mut result = vec![0; 20];
        result[..4].copy_from_slice(&20u32.to_le_bytes());
        result[16..20].copy_from_slice(&u32::from(wide).to_le_bytes());
        result
    }
    #[test]
    fn copied_file_lists_preserve_unicode_order_and_ansi_paths() {
        let mut wide = header(true);
        wide.extend(
            "C:\\画\\one.png\0D:\\two.jpg\0\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert_eq!(
            parse(&wide).unwrap(),
            [
                PathBuf::from("C:\\画\\one.png"),
                PathBuf::from("D:\\two.jpg")
            ]
        );
        let mut ansi = header(false);
        ansi.extend(b"C:\\one.png\0D:\\two.bmp\0\0");
        assert_eq!(parse(&ansi).unwrap().len(), 2);
        assert_eq!(
            parse(&[header(true), vec![0; 4]].concat()),
            Some(Vec::new())
        );
    }
    #[test]
    fn damaged_and_unbounded_file_lists_are_rejected_without_partial_results() {
        let mut valid = header(true);
        valid.extend(
            "C:\\one.png\0D:\\two.png\0\0"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        for size in 0..valid.len() - 1 {
            assert!(parse(&valid[..size]).is_none(), "truncated at {size}");
        }
        for offset in [0u32, 19, 21, u32::MAX] {
            let mut bad = valid.clone();
            bad[..4].copy_from_slice(&offset.to_le_bytes());
            assert!(parse(&bad).is_none());
        }
        let mut nonzero_bool = valid.clone();
        nonzero_bool[16..20].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(parse(&nonzero_bool), parse(&valid));
        let mut many = header(false);
        many.extend(b"a.png\0".repeat(MAX_FILES + 1));
        many.push(0);
        assert!(parse(&many).is_none());
        let mut long = header(false);
        long.extend(vec![b'a'; MAX_PATH + 1]);
        long.extend([0, 0]);
        assert!(parse(&long).is_none());
        assert!(parse(&vec![0; MAX_BYTES + 1]).is_none());
    }
}
