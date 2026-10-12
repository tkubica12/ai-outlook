#[cfg(windows)]
pub(crate) fn protect(data: &[u8], binding: &str, encrypt: bool) -> Result<Vec<u8>, String> {
    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }
    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    let input = Blob {
        size: u32::try_from(data.len()).map_err(|_| "Credential buffer is too large")?,
        data: data.as_ptr().cast_mut(),
    };
    let entropy = Blob {
        size: u32::try_from(binding.len()).map_err(|_| "Credential binding is too large")?,
        data: binding.as_ptr().cast_mut(),
    };
    let mut output = Blob {
        size: 0,
        data: std::ptr::null_mut(),
    };
    // UI is forbidden; without LOCAL_MACHINE, DPAPI binds to the current Windows user.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(
            "Windows could not protect or unlock this sign-in; reconnect from this Windows account"
                .into(),
        );
    }
    let value = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.data, output.size as usize);
        let value = slice.to_vec();
        slice.fill(0);
        LocalFree(output.data.cast());
        value
    };
    Ok(value)
}

#[cfg(not(windows))]
pub(crate) fn protect(_: &[u8], _: &str, _: bool) -> Result<Vec<u8>, String> {
    Err("Saved sign-ins require Windows DPAPI".into())
}
