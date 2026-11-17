#[allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    clippy::all
)]
pub mod Microsoft {
    pub mod Web {
        pub mod WebView2 {
            pub mod Win32 {
                mod windows_core {
                    // The loader is not an import of the program: each function is looked up when first
                    // called, from the DLL carried inside the program (see loader.rs).
                    macro_rules! link_webview2 {
                        ($library:literal $abi:literal fn $name:ident($($arg:ident : $argty:ty),* $(,)?) -> windows_core::HRESULT) => (
                            unsafe fn $name($($arg: $argty),*) -> windows_core::HRESULT {
                                let p = $crate::loader::symbol(concat!(stringify!($name), "\0"));
                                if p.is_null() {
                                    return $crate::loader::missing();
                                }
                                let f: unsafe extern $abi fn($($argty),*) -> windows_core::HRESULT =
                                    ::core::mem::transmute(p);
                                f($($arg),*)
                            }
                        )
                    }

                    pub(crate) use ::windows_core::*;
                    pub(crate) use link_webview2 as link;
                }

                include!("bindings.rs");
            }
        }
    }
}

mod loader;
pub mod declared_interfaces;

#[cfg(test)]
mod test {
    use windows_core::w;

    use crate::Microsoft::Web::WebView2::Win32::*;

    #[test]
    fn compare_eq() {
        let mut result = 1;
        unsafe { CompareBrowserVersions(w!("1.0.0"), w!("1.0.0"), &mut result) }.unwrap();
        assert_eq!(0, result);
    }

    #[test]
    fn compare_lt() {
        let mut result = 0;
        unsafe { CompareBrowserVersions(w!("1.0.0"), w!("1.0.1"), &mut result) }.unwrap();
        assert_eq!(-1, result);
    }

    #[test]
    fn compare_gt() {
        let mut result = 0;
        unsafe { CompareBrowserVersions(w!("2.0.0"), w!("1.0.1"), &mut result) }.unwrap();
        assert_eq!(1, result);
    }
}
