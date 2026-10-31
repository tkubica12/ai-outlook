/// Opens an HTTPS address in the user's default browser without a console or shell command.
pub fn open(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid sign-in address".to_string())?;
    if parsed.scheme() != "https" || url.chars().any(char::is_control) {
        return Err("Only HTTPS sign-in addresses can be opened".into());
    }
    shell_open(parsed.as_str(), "Could not open the browser")
}

/// Accepts only links built by `followup::mailto`: plain recipient addresses and percent-encoded
/// `subject` and `body` values; no other headers such as cc, bcc or attachments.
pub fn validate_mailto(uri: &str) -> Result<(), String> {
    let invalid =
        || "Only e-mail links with plain addresses, a subject and a body can be opened".to_string();
    if uri.len() > crate::followup::MAILTO_LIMIT
        || !uri.bytes().all(|byte| {
            byte.is_ascii_graphic() && !matches!(byte, b'#' | b'"' | b'<' | b'>' | b'\\')
        })
    {
        return Err(invalid());
    }
    let rest = uri.strip_prefix("mailto:").ok_or_else(invalid)?;
    let (to, query) = rest.split_once('?').ok_or_else(invalid)?;
    if !to.is_empty()
        && to
            .split(',')
            .any(|address| crate::calendar::person_email(address).as_deref() != Some(address))
    {
        return Err(invalid());
    }
    let mut names = std::collections::BTreeSet::new();
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').ok_or_else(invalid)?;
        let bytes = value.as_bytes();
        let encoded = bytes.iter().enumerate().all(|(index, byte)| match byte {
            b'%' => bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)),
            byte => byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'),
        });
        if !matches!(name, "subject" | "body") || !names.insert(name) || !encoded {
            return Err(invalid());
        }
    }
    if !names.contains("subject")
        || url::Url::parse(uri).map(|url| url.scheme() == "mailto") != Ok(true)
    {
        return Err(invalid());
    }
    Ok(())
}

/// Opens a validated `mailto:` link in the user's e-mail app after an explicit click. The app
/// shows a message the user may edit and send; Tomlook itself never sends anything.
pub fn open_mailto(uri: &str) -> Result<(), String> {
    validate_mailto(uri)?;
    shell_open(uri, "Could not open the e-mail app")
}

fn shell_open(target: &str, failure: &str) -> Result<(), String> {
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
        let (operation, file) = (wide("open"), wide(target));
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
            return Err(format!("{failure} (code {result})"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (target, failure);
        Err("Opening links currently requires Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_links_allow_only_plain_addresses_subject_and_body() {
        for valid in [
            "mailto:alex@contoso.example?subject=Hi",
            "mailto:alex@contoso.example,riley+pilot@northstar.example?subject=Hi%20there&body=Line%0D%0Aok",
            "mailto:?subject=S&body=B",
        ] {
            validate_mailto(valid).unwrap();
        }
        for invalid in [
            "https://example.com",
            "mailto:alex@contoso.example",
            "mailto:alex@contoso.example?subject=Hi&cc=x@evil.example",
            "mailto:alex@contoso.example?subject=Hi&attach=C:%5Cfile",
            "mailto:alex@contoso.example?body=only",
            "mailto:alex@contoso.example?subject=a&subject=b",
            "mailto:Alex%20<alex@contoso.example>?subject=Hi",
            "mailto:alex@contoso.example?subject=Hi there",
            "mailto:alex@contoso.example?subject=Hi%2",
            "mailto:alex@contoso.example?subject=Hi+there",
            "mailto:ALEX@contoso.example?subject=Hi",
            "mailto:alex@contoso.example?subject=Hi#fragment",
            "mailto:javascript:alert(1)?subject=Hi",
        ] {
            assert!(validate_mailto(invalid).is_err(), "{invalid}");
        }
        assert!(
            validate_mailto(&format!(
                "mailto:a@contoso.example?subject={}",
                "a".repeat(2_000)
            ))
            .is_err()
        );
        assert!(open_mailto("mailto:a@contoso.example?subject=Hi&bcc=x@evil.example").is_err());
    }
}
