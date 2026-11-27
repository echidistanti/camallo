//! Showing a file in the file manager on Linux: selected, as Explorer does with `/select`. The request goes
//! over D-Bus (`org.freedesktop.FileManager1.ShowItems`), which the common file managers answer; when nobody
//! does, the folder is opened instead.

use std::path::Path;

/// `file:///home/me/My%20Videos/a.mkv`: the address a file manager is asked to show.
fn file_uri(path: &Path) -> String {
    use std::fmt::Write;
    let mut out = String::from("file://");
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    for b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn show_item(path: &Path) -> zbus::Result<()> {
    zbus::blocking::Connection::session()?.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![file_uri(path)], ""),
    )?;
    Ok(())
}

/// Asks the file manager to show the file, selected; failing that, opens its folder. From a thread of its own:
/// a file manager that has to start first would hold the window up.
pub fn reveal(path: &Path) {
    let path = path.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("reveal".into())
        .spawn(move || {
            if show_item(&path).is_err() {
                crate::app::open_path(path.parent().unwrap_or(&path));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_becomes_an_address_a_file_manager_understands() {
        assert_eq!(
            file_uri(Path::new("/home/me/Videos/Camallo/a.mp4")),
            "file:///home/me/Videos/Camallo/a.mp4"
        );
        assert_eq!(
            file_uri(Path::new("/home/me/My Videos/100% #1 é.mp4")),
            "file:///home/me/My%20Videos/100%25%20%231%20%C3%A9.mp4"
        );
    }

    /// Needs a session bus. Run under `dbus-run-session`, with `dbus-monitor` watching, to see the call go out;
    /// no file manager is needed for that, the answer is an error that `reveal` turns into opening the folder.
    #[test]
    #[ignore = "needs a session bus"]
    fn the_request_reaches_the_bus() {
        let e = show_item(Path::new("/home/me/My Videos/a b.mp4"));
        eprintln!("answer: {e:?}");
    }
}
