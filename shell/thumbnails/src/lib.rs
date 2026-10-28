//! Windows Explorer thumbnails for .dudel projects. Explorer hands the file
//! as a stream; only its header is read, where Dudel stores a small preview
//! (see src/file_preview.rs). Layers are never decoded here. Preview dimensions
//! are capped before pixel decoding, with an additional decoder allocation budget.
//!
//! `regsvr32 dudel_thumbnails.dll` registers it for the current user (no
//! administrator rights); `regsvr32 /u` removes it.
#![cfg(windows)]
#![allow(non_snake_case)]

#[path = "../../../src/file_preview_header.rs"]
mod header;

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicIsize, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::{
            CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED,
            ERROR_FILE_NOT_FOUND, HINSTANCE, S_FALSE, S_OK,
        },
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
        },
        System::{
            Com::{IClassFactory, IClassFactory_Impl, IStream},
            LibraryLoader::GetModuleFileNameW,
            Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteTreeW, RegSetKeyValueW},
            SystemServices::DLL_PROCESS_ATTACH,
        },
        UI::Shell::{
            IThumbnailProvider, IThumbnailProvider_Impl,
            PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl},
            SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, WTS_ALPHATYPE, WTSAT_RGB,
        },
    },
    core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, Ref, Result, implement},
};

/// Class id of this provider; registry entries point Explorer at it.
pub const CLSID: GUID = GUID::from_u128(0x7fd5a7c1_d502_47b2_a703_086e549231ff);
const CLSID_TEXT: &str = "{7fd5a7c1-d502-47b2-a703-086e549231ff}";
/// Shell extension slot for IThumbnailProvider handlers.
const THUMBNAIL_SLOT: &str = "{e357fccd-a995-4576-b01f-234630154e96}";

static MODULE: AtomicIsize = AtomicIsize::new(0);

#[unsafe(no_mangle)]
extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module.0 as isize, Ordering::Relaxed);
    }
    true.into()
}

/// Preview image bytes (PNG or JPEG) from the first bytes of a file, scaled
/// to fit `side` and returned as top-down BGRA rows.
pub fn thumbnail(prefix: &[u8], side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(header::read(prefix)?))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(header::MAX_SIDE);
    limits.max_image_height = Some(header::MAX_SIDE);
    // Dimensions are strict; the decoder allocation allowance is best effort.
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let side = side.max(1);
    let image = if image.width() > side || image.height() > side {
        image.resize(side, side, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    let bgra = rgb.pixels().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    Some((width, height, bgra))
}

#[implement(IInitializeWithStream, IThumbnailProvider)]
struct Provider {
    prefix: Mutex<Option<Vec<u8>>>,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, stream: Ref<'_, IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?;
        let mut prefix = vec![0u8; header::PREFIX_LIMIT];
        let mut filled = 0;
        while filled < prefix.len() {
            let mut read = 0u32;
            let remaining = &mut prefix[filled..];
            let status = unsafe {
                stream.Read(
                    remaining.as_mut_ptr().cast(),
                    remaining.len() as u32,
                    Some(&mut read),
                )
            };
            status.ok()?;
            if read == 0 {
                break;
            }
            filled += read as usize;
        }
        prefix.truncate(filled);
        let mut slot = self.prefix.lock().map_err(|_| E_UNEXPECTED)?;
        if slot.is_some() {
            return Err(E_UNEXPECTED.into());
        }
        *slot = Some(prefix);
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(
        &self,
        side: u32,
        bitmap: *mut HBITMAP,
        alpha: *mut WTS_ALPHATYPE,
    ) -> Result<()> {
        if bitmap.is_null() || alpha.is_null() {
            return Err(E_POINTER.into());
        }
        let prefix = self.prefix.lock().map_err(|_| E_UNEXPECTED)?;
        let prefix = prefix.as_deref().ok_or(E_UNEXPECTED)?;
        // Decoders run on untrusted bytes; a panic must not unwind into COM.
        let (width, height, bgra) = catch_unwind(AssertUnwindSafe(|| thumbnail(prefix, side)))
            .ok()
            .flatten()
            .ok_or(E_FAIL)?;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        unsafe {
            let created = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            if bits.is_null() {
                return Err(E_FAIL.into());
            }
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast(), bgra.len());
            *bitmap = created;
            *alpha = WTSAT_RGB;
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *object = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider {
            prefix: Mutex::new(None),
        }
        .into();
        unsafe { provider.query(iid, object).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    unsafe {
        *object = std::ptr::null_mut();
        if *clsid != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

/// Keeping the DLL loaded is always safe; the thumbnail host exits on its own.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

fn set(key: &str, name: Option<&str>, value: &str) -> Result<()> {
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let name = name.map(HSTRING::from);
    let name = name
        .as_ref()
        .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            name,
            REG_SZ.0,
            Some(value.as_ptr().cast()),
            (value.len() * 2) as u32,
        )
        .ok()
    }
}

fn module_path() -> Result<String> {
    let mut buffer = vec![0u16; 32_768];
    let module = HINSTANCE(MODULE.load(Ordering::Relaxed) as *mut c_void);
    let length = unsafe { GetModuleFileNameW(Some(module.into()), &mut buffer) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(E_FAIL.into());
    }
    Ok(String::from_utf16_lossy(&buffer[..length]))
}

fn notify() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Per-user registration under HKCU\Software\Classes; no elevation needed.
#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    let result = (|| {
        let class = format!(r"Software\Classes\CLSID\{CLSID_TEXT}");
        set(&class, None, "Dudel project thumbnails")?;
        set(&format!(r"{class}\InprocServer32"), None, &module_path()?)?;
        set(
            &format!(r"{class}\InprocServer32"),
            Some("ThreadingModel"),
            "Apartment",
        )?;
        set(
            &format!(r"Software\Classes\.dudel\ShellEx\{THUMBNAIL_SLOT}"),
            None,
            CLSID_TEXT,
        )
    })();
    notify();
    result.map_or_else(|error| error.code(), |()| S_OK)
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    let mut status = S_OK;
    for key in [
        format!(r"Software\Classes\.dudel\ShellEx\{THUMBNAIL_SLOT}"),
        format!(r"Software\Classes\CLSID\{CLSID_TEXT}"),
    ] {
        let error = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
        if error.is_err() && error != ERROR_FILE_NOT_FOUND {
            status = error.to_hresult();
        }
    }
    notify();
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(width: u32, height: u32) -> Vec<u8> {
        project_image(width, height, image::ImageFormat::Png)
    }

    fn project_image(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(width, height, image::Rgb([10, 20, 30]))
            .write_to(&mut std::io::Cursor::new(&mut png), format)
            .unwrap();
        let mut file = b"DUDL\x04\r\n".to_vec();
        file.extend_from_slice(&[0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 255]);
        file.extend_from_slice(&((png.len() + 4) as u16).to_le_bytes());
        file.extend_from_slice(b"DP");
        file.extend_from_slice(&(png.len() as u16).to_le_bytes());
        file.extend_from_slice(&png);
        // Stand-in for the compressed layers that follow.
        file.extend(std::iter::repeat_n(7, 100_000));
        file
    }

    #[test]
    fn explorer_path_builds_a_bitmap_through_com() {
        use windows::Win32::{
            Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW},
            UI::Shell::SHCreateMemStream,
        };
        unsafe {
            let mut factory: Option<IClassFactory> = None;
            DllGetClassObject(&CLSID, &IClassFactory::IID, &mut factory as *mut _ as _).unwrap();
            let factory = factory.unwrap();
            let provider: IInitializeWithStream =
                factory.CreateInstance(None::<&IUnknown>).unwrap();
            let stream = SHCreateMemStream(Some(&project(256, 128))).unwrap();
            provider.Initialize(&stream, 0).unwrap();
            assert!(provider.Initialize(&stream, 0).is_err());
            let thumbnails: IThumbnailProvider = provider.cast().unwrap();
            let (mut bitmap, mut alpha) = (HBITMAP::default(), WTS_ALPHATYPE::default());
            thumbnails
                .GetThumbnail(128, &mut bitmap, &mut alpha)
                .unwrap();
            let mut info = BITMAP::default();
            GetObjectW(
                bitmap.into(),
                size_of::<BITMAP>() as i32,
                Some(&mut info as *mut _ as _),
            );
            assert_eq!(
                (info.bmWidth, info.bmHeight, info.bmBitsPixel),
                (128, 64, 32)
            );
            assert_eq!(alpha, WTSAT_RGB);
            let _ = DeleteObject(bitmap.into());
            let other = factory
                .CreateInstance::<_, IInitializeWithStream>(None::<&IUnknown>)
                .unwrap();
            other
                .Initialize(&SHCreateMemStream(Some(b"CSFCHUNK")).unwrap(), 0)
                .unwrap();
            let other: IThumbnailProvider = other.cast().unwrap();
            assert!(other.GetThumbnail(128, &mut bitmap, &mut alpha).is_err());
            let wrong = GUID::from_u128(1);
            assert_eq!(
                DllGetClassObject(&wrong, &IClassFactory::IID, &mut std::ptr::null_mut()),
                CLASS_E_CLASSNOTAVAILABLE
            );
        }
    }

    #[test]
    fn previews_scale_to_the_requested_side_and_other_files_are_refused() {
        let file = project(200, 100);
        let (width, height, bgra) = thumbnail(&file, 96).unwrap();
        assert_eq!((width, height), (96, 48));
        assert_eq!(&bgra[..4], &[30, 20, 10, 255]);
        let (width, _, _) = thumbnail(&file, 1024).unwrap();
        assert_eq!(width, 200);
        assert!(thumbnail(&file[..40], 96).is_none());
        assert!(thumbnail(b"CSFCHUNK", 96).is_none());
    }

    #[test]
    fn png_and_jpeg_previews_enforce_the_writer_dimensions_before_resizing() {
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let file = project_image(header::MAX_SIDE, header::MAX_SIDE, format);
            let (width, height, _) = thumbnail(&file, u32::MAX).unwrap();
            assert_eq!((width, height), (256, 256));
            assert!(thumbnail(&file, 0).is_some());
            // These small compressed fixtures used to decode and resize successfully.
            // A small requested thumbnail must not bypass the source dimension limit.
            for (width, height) in [(257, 1), (1, 257), (512, 512)] {
                let file = project_image(width, height, format);
                assert!(thumbnail(&file, 32).is_none());
            }
        }
    }
}
