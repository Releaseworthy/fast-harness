//! The Windows "Select folder" dialog: IFileOpenDialog through raw COM vtables.
//! windows-sys has no COM interface types, so the two vtables are declared here in the
//! order of shobjidl_core.h (checked against the `windows` 0.62.2 crate, 2026-11-12):
//! IUnknown, IModalWindow::Show, IFileDialog (23 methods), IFileOpenDialog (2 methods).

use std::ffi::c_void;
use std::path::PathBuf;

use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::UI::Shell::{
    FileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

use crate::platform::PlatformError;

/// IID_IFileOpenDialog.
const IID_FILE_OPEN_DIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
/// HRESULT_FROM_WIN32(ERROR_CANCELLED): the user closed the dialog.
pub(crate) const CANCELLED: HRESULT = 0x800704C7_u32 as HRESULT;
/// S_FALSE from CoInitializeEx: COM was already initialised on this thread (still counted).
const S_FALSE: HRESULT = 1;

type Slot = usize;

#[repr(C)]
struct UnknownVtbl {
    query_interface: Slot,
    add_ref: Slot,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct DialogVtbl {
    query_interface: Slot,
    add_ref: Slot,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    show: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    set_file_types: Slot,
    set_file_type_index: Slot,
    get_file_type_index: Slot,
    advise: Slot,
    unadvise: Slot,
    set_options: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    set_default_folder: Slot,
    set_folder: Slot,
    get_folder: Slot,
    get_current_selection: Slot,
    set_file_name: Slot,
    get_file_name: Slot,
    set_title: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    set_ok_button_label: Slot,
    set_file_name_label: Slot,
    get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    add_place: Slot,
    set_default_extension: Slot,
    close: Slot,
    set_client_guid: Slot,
    clear_client_data: Slot,
    set_filter: Slot,
    get_results: Slot,
    get_selected_items: Slot,
}

#[repr(C)]
struct ShellItemVtbl {
    query_interface: Slot,
    add_ref: Slot,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    bind_to_handler: Slot,
    get_parent: Slot,
    get_display_name: unsafe extern "system" fn(*mut c_void, i32, *mut PWSTR) -> HRESULT,
    get_attributes: Slot,
    compare: Slot,
}

/// Maps a dialog HRESULT: success continues, cancel is `Ok(None)`, anything else an error.
pub(crate) fn check(hr: HRESULT) -> Result<Option<()>, PlatformError> {
    match hr {
        0.. => Ok(Some(())),
        CANCELLED => Ok(None),
        _ => Err(PlatformError::Os {
            code: hr,
            message: format!("HRESULT 0x{:08X}", hr as u32),
        }),
    }
}

/// Owns one COM reference; `Release` runs on drop, on every path.
struct Com<V> {
    ptr: *mut c_void,
    _vtbl: std::marker::PhantomData<V>,
}

impl<V> Com<V> {
    fn vtbl(&self) -> &V {
        // SAFETY: `ptr` is a live COM object whose first field points at a vtable that
        // starts with the layout `V` (the interface asked for when the pointer was made).
        unsafe { &**(self.ptr as *const *const V) }
    }
}

impl<V> Drop for Com<V> {
    fn drop(&mut self) {
        // SAFETY: every vtable starts with IUnknown, whose third slot is Release; this
        // wrapper holds exactly one reference.
        unsafe {
            let unknown = &**(self.ptr as *const *const UnknownVtbl);
            (unknown.release)(self.ptr);
        }
    }
}

/// Balances CoInitializeEx with CoUninitialize on drop.
struct Apartment;

impl Apartment {
    fn enter() -> Result<Apartment, PlatformError> {
        // SAFETY: plain call; the reserved pointer must be null.
        let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        match hr {
            0 | S_FALSE => Ok(Apartment),
            _ => check(hr).map(|_| Apartment),
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: matches the successful CoInitializeEx of `enter` on this thread.
        unsafe { CoUninitialize() }
    }
}

/// Shows the modal "Select folder" dialog owned by `owner` (0 for none) and returns the
/// picked folder, or `Ok(None)` when the user cancels. Blocks the calling thread; call it
/// on a thread of its own, not on the UI thread.
pub fn pick_folder(owner: isize, title: &str) -> Result<Option<PathBuf>, PlatformError> {
    let _apartment = Apartment::enter()?;
    let dialog = create_dialog()?;
    configure(&dialog, title)?;
    // SAFETY: `dialog` is a live IFileOpenDialog; `owner` is a window handle or null.
    let hr = unsafe { (dialog.vtbl().show)(dialog.ptr, owner as HWND) };
    if check(hr)?.is_none() {
        return Ok(None);
    }
    let mut item: *mut c_void = std::ptr::null_mut();
    // SAFETY: `item` receives one IShellItem reference, owned by `Com` below.
    check(unsafe { (dialog.vtbl().get_result)(dialog.ptr, &mut item) })?;
    let item: Com<ShellItemVtbl> = Com {
        ptr: item,
        _vtbl: std::marker::PhantomData,
    };
    item_path(&item).map(Some)
}

fn create_dialog() -> Result<Com<DialogVtbl>, PlatformError> {
    let mut ptr: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid CLSID and IID constants; `ptr` receives one reference on success.
    let hr = unsafe {
        CoCreateInstance(
            &FileOpenDialog,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_FILE_OPEN_DIALOG,
            &mut ptr,
        )
    };
    check(hr)?;
    Ok(Com {
        ptr,
        _vtbl: std::marker::PhantomData,
    })
}

fn configure(dialog: &Com<DialogVtbl>, title: &str) -> Result<(), PlatformError> {
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let mut options = 0u32;
    // SAFETY: live dialog; `options` and the NUL-terminated `wide` outlive the calls.
    unsafe {
        check((dialog.vtbl().get_options)(dialog.ptr, &mut options))?;
        let wanted = options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        check((dialog.vtbl().set_options)(dialog.ptr, wanted))?;
        check((dialog.vtbl().set_title)(dialog.ptr, wide.as_ptr()))?;
    }
    Ok(())
}

fn item_path(item: &Com<ShellItemVtbl>) -> Result<PathBuf, PlatformError> {
    let mut name: PWSTR = std::ptr::null_mut();
    // SAFETY: live IShellItem; on success `name` is a NUL-terminated CoTaskMem string,
    // read once and then freed with CoTaskMemFree.
    unsafe {
        check((item.vtbl().get_display_name)(
            item.ptr,
            SIGDN_FILESYSPATH,
            &mut name,
        ))?;
        let mut len = 0;
        while *name.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(name, len));
        CoTaskMemFree(name as *const c_void);
        Ok(PathBuf::from(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hresults_map_to_continue_cancel_or_error() {
        assert_eq!(check(0), Ok(Some(())));
        assert_eq!(check(1), Ok(Some(())));
        assert_eq!(check(CANCELLED), Ok(None));
        assert_eq!(CANCELLED as u32, 0x800704C7);
        let err = check(0x80004005_u32 as i32).unwrap_err();
        assert_eq!(
            err,
            PlatformError::Os {
                code: 0x80004005_u32 as i32,
                message: "HRESULT 0x80004005".into()
            }
        );
    }
}
