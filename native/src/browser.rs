/// Opens an HTTPS address in the user's default browser without a console or shell command.
pub fn open(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid sign-in address".to_string())?;
    if parsed.scheme() != "https" || url.chars().any(char::is_control) {
        return Err("Only HTTPS sign-in addresses can be opened".into());
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (operation, file) = (wide("open"), wide(parsed.as_str()));
        // Both terminated buffers outlive the synchronous call; SW_SHOWNORMAL = 1.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result <= 32 {
            return Err(format!("Could not open the browser (code {result})"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Opening the browser currently requires Windows".into())
    }
}
