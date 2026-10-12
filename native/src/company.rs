use eframe::egui::ColorImage;
use std::{net::IpAddr, path::Path, time::Duration};

const BYTE_LIMIT: usize = 131_072;

pub fn valid_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.parse::<IpAddr>().is_err()
        && ![".localhost", ".local", ".internal", ".test", ".invalid"]
            .iter()
            .any(|suffix| domain.ends_with(suffix))
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !(a == 192 && b == 0)
        }
        IpAddr::V6(ip) => {
            let parts = ip.segments();
            (parts[0] & 0xe000) == 0x2000
                && parts[0] != 0x2002
                && !(parts[0] == 0x2001 && matches!(parts[1], 0 | 2 | 0x10 | 0xdb8))
        }
    }
}

fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    if bytes.len() > BYTE_LIMIT {
        return Err("Company icon exceeds its size limit".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Company icon format is unsupported")?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Ico)
    ) {
        return Err("Company icon must be PNG, JPEG or ICO".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(2_097_152);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "Company icon could not be decoded within its limits")?
        .thumbnail(32, 32)
        .to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

pub async fn load(root: &Path, domain: &str) -> Result<Option<ColorImage>, String> {
    if !valid_domain(domain) {
        return Err("Company icon requires a public company domain".into());
    }
    let directory = root.join("company-icons");
    let cache = directory.join(format!("{domain}.icon"));
    let cached = cache.clone();
    if let Some(image) = tokio::task::spawn_blocking(move || match std::fs::metadata(&cached) {
        Ok(metadata) => {
            if metadata.len() > BYTE_LIMIT as u64 {
                return Err("Cached company icon exceeds its size limit".to_string());
            }
            let bytes = std::fs::read(cached).map_err(|_| "Read company icon cache failed")?;
            decode(&bytes).map(Some)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Company icon cache is unavailable".into()),
    })
    .await
    .map_err(|_| "Company icon cache worker stopped")??
    {
        return Ok(Some(image));
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        for host in [domain.to_owned(), format!("www.{domain}")] {
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 443))
                .await
                .map_err(|_| "Company website address lookup failed")?
                .collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !public_address(address.ip()))
            {
                return Err("Company icon lookup refused a non-public website address".into());
            }
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(&host, &addresses)
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(8))
                .build()
                .map_err(|_| "Create company icon connection failed")?;
            // Only the company domain goes to its own HTTPS website; never an email or meeting.
            let mut response = client
                .get(format!("https://{host}/favicon.ico"))
                .send()
                .await
                .map_err(|_| "Company website icon request failed")?;
            if response.status().is_redirection()
                || response.status() == reqwest::StatusCode::NOT_FOUND
            {
                continue;
            }
            if !response.status().is_success() {
                return Err("Company website refused its icon request".into());
            }
            if response
                .content_length()
                .is_some_and(|length| length > BYTE_LIMIT as u64)
            {
                return Err("Company icon exceeds its size limit".into());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Company icon download failed")?
            {
                if bytes.len() + chunk.len() > BYTE_LIMIT {
                    return Err("Company icon exceeds its size limit".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            return tokio::task::spawn_blocking(move || {
                let image = decode(&bytes)?;
                std::fs::create_dir_all(directory)
                    .map_err(|_| "Create company icon cache failed")?;
                std::fs::write(cache, bytes).map_err(|_| "Save company icon cache failed")?;
                Ok(Some(image))
            })
            .await
            .map_err(|_| "Company icon decoding worker stopped")?;
        }
        Ok(None)
    })
    .await
    .map_err(|_| "Company icon lookup timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_hosts_cannot_be_paths_addresses_or_private_networks() {
        for domain in [
            "localhost",
            "127.0.0.1",
            "company.local",
            "a.test",
            "a/b.com",
            "a.com:443",
            "a@b.com",
            "-a.com",
            "a..com",
        ] {
            assert!(!valid_domain(domain), "{domain}");
        }
        for address in [
            "127.0.0.1",
            "10.1.1.1",
            "172.16.1.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.1.1",
            "198.18.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        assert!(valid_domain("microsoft.com"));
        assert!(public_address("1.1.1.1".parse().unwrap()));
        assert!(public_address("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn cached_official_icon_is_bounded_and_decoded_without_network() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("company-icons");
        std::fs::create_dir(&folder).unwrap();
        let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([50, 160, 240, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Ico).unwrap();
        std::fs::write(folder.join("fixture.example.icon"), bytes.into_inner()).unwrap();
        assert_eq!(
            load(root.path(), "fixture.example")
                .await
                .unwrap()
                .unwrap()
                .size,
            [32, 32]
        );
        assert!(decode(&vec![0; BYTE_LIMIT + 1]).is_err());
        assert!(decode(b"<svg>not a supported raster icon</svg>").is_err());
    }
}
