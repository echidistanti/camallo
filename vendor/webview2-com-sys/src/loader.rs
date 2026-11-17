//! The WebView2 loader DLL, carried inside the program.
//! It is written once to a per-user folder and loaded from there, so the program is a single file.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::OnceLock;

#[cfg(target_arch = "x86_64")]
static DLL: &[u8] = include_bytes!("../x64/WebView2Loader.dll");

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// Where the copy lives; the size is in the name, so a different build of the DLL never meets an old copy.
fn target() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Camallo").join("runtime").join(format!("WebView2Loader-{}.dll", DLL.len()))
}

fn complete(p: &std::path::Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() == DLL.len() as u64).unwrap_or(false)
}

/// Writes the DLL if it is not there yet. Two processes may do it at once (the window and the browser):
/// each writes its own file and renames it, and whoever loses finds the finished copy already in place.
fn extract() -> std::io::Result<PathBuf> {
    let to = target();
    if complete(&to) {
        return Ok(to);
    }
    let dir = to.parent().expect("the target has a folder");
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!("WebView2Loader.{}.part", std::process::id()));
    std::fs::write(&part, DLL)?;
    let moved = std::fs::rename(&part, &to);
    if moved.is_err() {
        let _ = std::fs::remove_file(&part);
        if !complete(&to) {
            return moved.map(|_| to);
        }
    }
    Ok(to)
}

fn module() -> usize {
    static MODULE: OnceLock<usize> = OnceLock::new();
    *MODULE.get_or_init(|| {
        let Ok(path) = extract() else { return 0 };
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { LoadLibraryW(name.as_ptr()) as usize }
    })
}

/// Address of a function of the loader; `name` ends with a zero byte. Null when the DLL could not be prepared.
pub fn symbol(name: &str) -> *mut c_void {
    debug_assert!(name.ends_with('\0'));
    match module() {
        0 => std::ptr::null_mut(),
        m => unsafe { GetProcAddress(m as *mut c_void, name.as_ptr()) },
    }
}

/// What the five functions answer when the loader is not there: ERROR_MOD_NOT_FOUND as an HRESULT.
pub fn missing() -> windows_core::HRESULT {
    windows_core::HRESULT(0x8007007E_u32 as i32)
}
