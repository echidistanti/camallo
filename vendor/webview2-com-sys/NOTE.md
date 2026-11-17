# webview2-com-sys, adapted for Camallo

This is webview2-com-sys 0.39.1 (MIT, https://github.com/wravery/webview2-rs) with two changes.

On the windows-gnu target the upstream crate links `WebView2Loader.dll` as an import, so the program
refuses to start unless the DLL sits next to it. Here the five functions of the loader are looked up at the
moment they are first called (`src/lib.rs`, `src/loader.rs`), and the DLL (`x64/WebView2Loader.dll`, from the
same crate, signed by Microsoft) is carried inside the program and written to a per-user folder when needed.

`build.rs` only links advapi32; the import libraries of the upstream crate are not needed.
