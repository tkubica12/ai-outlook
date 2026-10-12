use super::Connection;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::AtomicUsize;

pub const SDK_TOOL: &str = "tomlook_cloud_read";
const FIELDS: &str =
    "id,name,webUrl,size,file,folder,parentReference,remoteItem,lastModifiedDateTime";
const FILE_LIMIT: usize = 262_144;
const CHUNK_LIMIT: usize = 8000;
const CHILD_LIMIT: usize = 100;

pub(super) struct Root {
    url: url::Url,
    drive_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arguments {
    pub path: String,
    #[serde(default)]
    pub offset: usize,
}

impl Arguments {
    pub fn validate(&self) -> Result<(), String> {
        validate_path(&self.path, true)?;
        if self.offset > FILE_LIMIT {
            return Err("Cloud text offset exceeds the 256 KiB file limit".into());
        }
        Ok(())
    }
}

fn validate_path(path: &str, empty: bool) -> Result<(), String> {
    if path.len() > 2000
        || (!empty && path.is_empty())
        || path
            .chars()
            .any(|c| c.is_control() || "\\%?#:*<>|".contains(c))
        || (!path.is_empty()
            && path
                .split('/')
                .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(['.', ' '])))
    {
        return Err(
            "Cloud paths must stay relative to the chosen folder; no traversal or actions".into(),
        );
    }
    Ok(())
}

pub fn validate_folder(folder: &str) -> Result<(), String> {
    if folder.is_empty() || folder.len() > 256 {
        return Err("Choose a cloud folder name or a short original OneDrive folder URL".into());
    }
    if folder.starts_with("https://") {
        original_folder_url(folder).map(|_| ())
    } else {
        validate_path(folder, false)
    }
}

fn original_folder_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|_| "Invalid OneDrive folder URL")?;
    if !crate::evidence::safe_source_url(value)
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with("-my.sharepoint.com"))
        || !url.path().starts_with("/personal/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an original personal OneDrive folder URL without query parameters".into());
    }
    Ok(url)
}

fn relative_folder(selected: &url::Url, root: &url::Url) -> Result<String, String> {
    if selected.origin() != root.origin() {
        return Err("The chosen folder is not inside the signed-in account's OneDrive".into());
    }
    let suffix = selected
        .path()
        .strip_prefix(&format!("{}/", root.path().trim_end_matches('/')))
        .ok_or("The chosen folder is not inside the signed-in account's OneDrive")?;
    let mut parts = Vec::new();
    for part in suffix.trim_end_matches('/').split('/') {
        let encoded = format!("part={}", part.replace('+', "%2B"));
        let decoded = url::form_urlencoded::parse(encoded.as_bytes())
            .next()
            .ok_or("Invalid cloud path encoding")?
            .1
            .into_owned();
        if decoded.contains('/') || decoded.contains('\u{fffd}') {
            return Err("Cloud path encoding contains an invalid segment".into());
        }
        validate_path(&decoded, false)?;
        parts.push(decoded);
    }
    let relative = parts.join("/");
    validate_path(&relative, false)?;
    Ok(relative)
}

fn resource(folder: &str, path: &str) -> Result<String, String> {
    validate_path(folder, false)?;
    validate_path(path, true)?;
    let mut url = url::Url::parse("https://graph.microsoft.com/")
        .map_err(|_| "Invalid fixed cloud resource")?;
    url.path_segments_mut()
        .map_err(|_| "Invalid fixed cloud path")?
        .clear()
        .extend(folder.split('/'))
        .extend(path.split('/').filter(|part| !part.is_empty()));
    Ok(format!("/me/drive/root:{}?$select={FIELDS}", url.path()))
}

fn children_resource(folder: &str, path: &str) -> Result<String, String> {
    let metadata = resource(folder, path)?;
    let (path, _) = metadata
        .split_once('?')
        .ok_or("Invalid fixed cloud metadata resource")?;
    Ok(format!("{path}:/children?$top={CHILD_LIMIT}"))
}

fn item_id(item: &Value) -> Result<&str, String> {
    item["id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .ok_or_else(|| "Cloud metadata has no valid own-drive item identity".into())
}

fn verify_parent(item: &Value, drive: &str, folder: &str, path: &str) -> Result<(), String> {
    if item["parentReference"]["driveId"].as_str() != Some(drive) {
        return Err("Cloud item is outside the signed-in account's own drive".into());
    }
    let parent = item["parentReference"]["path"]
        .as_str()
        .ok_or("Cloud item has no verifiable parent path")?;
    let suffix = parent
        .strip_prefix("/drive/root:")
        .or_else(|| parent.strip_prefix(&format!("/drives/{drive}/root:")))
        .ok_or("Cloud item has an unexpected parent path")?;
    let encoded = format!("path={}", suffix.replace('+', "%2B"));
    let decoded = url::form_urlencoded::parse(encoded.as_bytes())
        .next()
        .ok_or("Cloud item has an invalid parent path")?
        .1
        .into_owned();
    let full = if path.is_empty() {
        folder.to_owned()
    } else {
        format!("{folder}/{path}")
    };
    let expected = full.rsplit_once('/').map_or("", |(parent, _)| parent);
    if decoded.trim_start_matches('/') != expected {
        return Err(
            "Cloud item parent escaped the chosen folder path; shortcuts are not followed".into(),
        );
    }
    Ok(())
}

fn source_url(item: &Value) -> Result<&str, String> {
    item["webUrl"]
        .as_str()
        .filter(|value| {
            crate::evidence::safe_source_url(value)
                && url::Url::parse(value).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.port_or_known_default() == Some(443)
                        && url
                            .host_str()
                            .is_some_and(|host| host.ends_with("-my.sharepoint.com"))
                })
        })
        .ok_or_else(|| "Cloud metadata has no safe original OneDrive URL".into())
}

fn extension(name: &str) -> Result<&str, String> {
    name.rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| ["html", "htm", "md", "txt"].iter().any(|known| ext.eq_ignore_ascii_case(known)))
        .ok_or_else(|| "Cloud content reads support UTF-8 HTML, Markdown and text; other formats need semantic WorkIQ research".into())
}

fn decode_text(blob: Value, name: &str) -> Result<String, String> {
    use base64::Engine;
    let ext = extension(name)?;
    let status = blob["statusCode"]
        .as_u64()
        .ok_or("Cloud content has no HTTP status")?;
    if status != 200 {
        return Err(format!("Cloud content read returned HTTP {status}"));
    }
    let mime = blob["contentType"]
        .as_str()
        .ok_or("Cloud content has no MIME type")?;
    if !mime.starts_with("text/") && !mime.starts_with("application/octet-stream") {
        return Err("Cloud content is not a supported text format".into());
    }
    let size = blob["sizeBytes"]
        .as_u64()
        .ok_or("Cloud content has no byte count")?;
    let encoded = blob["base64Content"]
        .as_str()
        .ok_or("Cloud content has no encoded bytes")?;
    if size > FILE_LIMIT as u64 || encoded.len() > FILE_LIMIT.div_ceil(3) * 4 {
        return Err("Cloud content exceeds the 256 KiB text-file limit".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Cloud content has invalid Base64 encoding")?;
    if bytes.len() as u64 != size {
        return Err("Cloud content byte count does not match its metadata".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Cloud content is not UTF-8 text")?;
    if crate::context::credential_fields(&text) {
        return Err("Cloud content contains credential fields; its contents were withheld".into());
    }
    Ok(
        if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
            crate::html_text::extract(&text)
        } else {
            text
        },
    )
}

fn text_result(item: &Value, folder: &str, args: &Arguments, text: &str) -> Result<Value, String> {
    if args.offset > text.len() || !text.is_char_boundary(args.offset) {
        return Err("Cloud text offset is outside the returned UTF-8 document".into());
    }
    let end = text.floor_char_boundary((args.offset + CHUNK_LIMIT).min(text.len()));
    Ok(json!({
        "_cloud_source": {"url": source_url(item)?, "folder": folder, "path": args.path, "offset": args.offset,
            "modified_at":item["lastModifiedDateTime"]},
        "content": &text[args.offset..end],
        "next_offset": if end < text.len() { Some(end) } else { None },
        "text_bytes": text.len(),
        "read_at": chrono::Utc::now().to_rfc3339(),
        "scripts_executed": false,
    }))
}

fn folder_result(item: &Value, folder: &str, path: &str, data: Value) -> Result<Value, String> {
    let rows = data["value"]
        .as_array()
        .ok_or("Cloud folder read returned no children collection")?;
    if rows.len() > CHILD_LIMIT {
        return Err("Cloud folder read exceeds its 100-child limit".into());
    }
    let parent = item_id(item)?;
    let drive = item["parentReference"]["driveId"]
        .as_str()
        .ok_or("Cloud folder has no drive identity")?;
    let mut children = Vec::new();
    let mut excluded = 0;
    let mut ids = std::collections::BTreeSet::new();
    for row in rows {
        if row.get("remoteItem").is_some_and(|value| !value.is_null()) {
            excluded += 1;
            continue;
        }
        let id = item_id(row)?;
        if !ids.insert(id)
            || row["parentReference"]["id"].as_str() != Some(parent)
            || row["parentReference"]["driveId"].as_str() != Some(drive)
        {
            return Err("Cloud children do not belong to the requested own-drive folder".into());
        }
        let name = row["name"].as_str().ok_or("Cloud child has no name")?;
        validate_path(name, false)?;
        if name.contains('/') {
            return Err("Cloud child has an invalid path segment".into());
        }
        children.push(json!({
            "name": name,
            "path": if path.is_empty() { name.to_owned() } else { format!("{path}/{name}") },
            "url": source_url(row)?,
            "folder": row["folder"].is_object(),
            "readable_text": row["file"].is_object() && extension(name).is_ok(),
            "size_bytes": row["size"],
            "modified_at": row["lastModifiedDateTime"],
        }));
    }
    let complete = item["folder"]["childCount"].as_u64() == Some(rows.len() as u64)
        && data.get("@odata.nextLink").is_none();
    Ok(json!({
        "_cloud_source": {"url":source_url(item)?, "folder":folder, "path":path},
        "children": children,
        "excluded_shortcuts": excluded,
        "direct_children_complete": complete,
        "coverage": "Only this folder's direct children; no recursive content coverage. WorkIQ blocks skip/skiptoken pagination.",
        "read_at": chrono::Utc::now().to_rfc3339(),
    }))
}

impl Connection {
    async fn cloud_fetch(&self, path: &str, remaining: &AtomicUsize) -> Result<Value, String> {
        crate::copilot::reserve_workiq_read(remaining)?;
        self.fetch_resource_in(path, &self.admission.research).await
    }

    pub async fn cloud_read(
        &self,
        folder: &str,
        args: Arguments,
        remaining: &AtomicUsize,
    ) -> Result<Value, String> {
        validate_folder(folder)?;
        args.validate()?;
        if !self.is_active() {
            return Err("WorkIQ is disconnected; no cloud request was sent".into());
        }
        let (relative, drive) = {
            let mut cached = self.cloud_root_url.lock().await;
            if cached.is_none() {
                let item = self
                    .cloud_fetch("/me/drive/root?$select=webUrl,parentReference", remaining)
                    .await?;
                *cached = Some(Root {
                    url: original_folder_url(source_url(&item)?)?,
                    drive_id: item["parentReference"]["driveId"]
                        .as_str()
                        .ok_or("Cloud root has no drive identity")?
                        .to_owned(),
                });
            }
            let root = cached.as_ref().ok_or("Cloud root was not resolved")?;
            let relative = if folder.starts_with("https://") {
                relative_folder(&original_folder_url(folder)?, &root.url)?
            } else {
                folder.to_owned()
            };
            (relative, root.drive_id.clone())
        };
        let item = self
            .cloud_fetch(&resource(&relative, &args.path)?, remaining)
            .await?;
        if item.get("remoteItem").is_some_and(|value| !value.is_null()) {
            return Err(
                "Cloud shortcuts outside the chosen own-drive folder are not followed".into(),
            );
        }
        verify_parent(&item, &drive, &relative, &args.path)?;
        let id = item_id(&item)?;
        source_url(&item)?;
        if item["folder"].is_object() {
            if args.offset != 0 {
                return Err("Cloud folders do not support text offsets".into());
            }
            let data = self
                .cloud_fetch(&children_resource(&relative, &args.path)?, remaining)
                .await?;
            folder_result(&item, folder, &args.path, data)
        } else {
            let name = item["name"]
                .as_str()
                .ok_or("Cloud file has no name")?
                .to_owned();
            extension(&name)?;
            if !item["file"].is_object()
                || !item["size"]
                    .as_u64()
                    .is_some_and(|size| size <= FILE_LIMIT as u64)
            {
                return Err("Cloud file metadata does not describe a bounded text file".into());
            }
            crate::copilot::reserve_workiq_read(remaining)?;
            let blob = self
                .read_tool(
                    "fetch_blob",
                    json!({"path":format!("/me/drive/items/{id}/content")}),
                )
                .await?;
            let text = tokio::task::spawn_blocking(move || decode_text(blob, &name))
                .await
                .map_err(|_| "Cloud text worker stopped")??;
            text_result(&item, folder, &args, &text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Value {
        json!({"id":"fixture","name":"notes.html","webUrl":"https://example-my.sharepoint.com/personal/owner/Documents/Customer/notes.html","parentReference":{"id":"parent","driveId":"drive"},"file":{},"size":1})
    }

    #[test]
    fn cloud_paths_cannot_escape_the_selected_folder_or_supply_graph_actions() {
        for path in [
            "/outside",
            "../outside",
            "a/../b",
            "a//b",
            "a\\b",
            "%2e%2e",
            "a:content",
            "a?secret",
            ".env",
            "a/",
        ] {
            assert!(validate_path(path, true).is_err(), "{path}");
        }
        assert!(validate_path("", true).is_ok());
        assert!(
            resource("Customer", "Workshop/demo.html")
                .unwrap()
                .starts_with("/me/drive/root:/Customer/Workshop/demo.html?")
        );
        assert_eq!(
            children_resource("Customer", "Workshop").unwrap(),
            "/me/drive/root:/Customer/Workshop:/children?$top=100"
        );
        assert!(
            Arguments {
                path: "notes.html".into(),
                offset: FILE_LIMIT + 1
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn original_folder_urls_are_bound_to_the_verified_drive_root() {
        let root =
            original_folder_url("https://example-my.sharepoint.com/personal/owner/Documents")
                .unwrap();
        let selected = original_folder_url(
            "https://example-my.sharepoint.com/personal/owner/Documents/Customer%20Projects",
        )
        .unwrap();
        assert_eq!(
            relative_folder(&selected, &root).unwrap(),
            "Customer Projects"
        );
        for value in [
            "https://other-my.sharepoint.com/personal/owner/Documents/Customer",
            "https://example-my.sharepoint.com/personal/other/Documents/Customer",
            "https://example-my.sharepoint.com/personal/owner/Documents/Customer%2F..",
            "https://example-my.sharepoint.com/personal/owner/Documents/%252e%252e",
        ] {
            assert!(relative_folder(&original_folder_url(value).unwrap(), &root).is_err());
        }
        assert!(
            validate_folder(
                "https://example-my.sharepoint.com/personal/owner/Documents/Customer?token=secret"
            )
            .is_err()
        );
    }

    #[test]
    fn cloud_text_is_bounded_utf8_and_preserves_original_provenance() {
        use base64::Engine;
        let html = "<h1>Meeting &amp; demo</h1><script>untrusted()</script><p>Readable</p>";
        let blob = json!({"statusCode":200,"contentType":"text/html","sizeBytes":html.len(),"base64Content":base64::engine::general_purpose::STANDARD.encode(html)});
        assert_eq!(
            decode_text(blob, "notes.html").unwrap(),
            "Meeting & demo\nReadable"
        );
        let args = Arguments {
            path: "notes.html".into(),
            offset: 0,
        };
        let text = "\u{010d}".repeat(6000);
        let result = text_result(&item(), "Customer", &args, &text).unwrap();
        assert_eq!(result["content"].as_str().unwrap().len(), CHUNK_LIMIT);
        assert_eq!(result["next_offset"], CHUNK_LIMIT);
        assert_eq!(result["_cloud_source"]["url"], item()["webUrl"]);
        assert!(text_result(&item(), "Customer", &Arguments { offset: 1, ..args }, &text).is_err());
        assert!(decode_text(json!({"statusCode":403}), "notes.html").is_err());
        assert!(decode_text(json!({"statusCode":200,"contentType":"text/html","sizeBytes":FILE_LIMIT+1,"base64Content":""}), "notes.html").is_err());
    }

    #[test]
    fn folder_results_require_real_parentage_and_never_expose_download_urls() {
        let mut folder = item();
        folder["folder"] = json!({"childCount":1});
        let mut child = item();
        child["id"] = json!("child");
        child["parentReference"]["id"] = folder["id"].clone();
        child["@microsoft.graph.downloadUrl"] = json!("https://download.example/?token=secret");
        let result =
            folder_result(&folder, "Customer", "", json!({"value":[child.clone()]})).unwrap();
        assert_eq!(result["direct_children_complete"], true);
        assert!(!result.to_string().contains("secret"));
        assert_eq!(
            folder_result(
                &folder,
                "Customer",
                "",
                json!({"value":[child.clone()],"@odata.nextLink":"opaque"})
            )
            .unwrap()["direct_children_complete"],
            false
        );
        child["parentReference"]["id"] = json!("outside");
        assert!(folder_result(&folder, "Customer", "", json!({"value":[child]})).is_err());
    }

    #[test]
    fn target_parentage_blocks_same_drive_shortcuts_and_other_drives() {
        let mut target = item();
        target["parentReference"]["path"] = json!("/drive/root:/Customer");
        assert!(verify_parent(&target, "drive", "Customer", "notes.html").is_ok());
        target["parentReference"]["path"] = json!("/drive/root:/Outside");
        assert!(verify_parent(&target, "drive", "Customer", "notes.html").is_err());
        target["parentReference"]["path"] = json!("/drive/root:/Customer");
        target["parentReference"]["driveId"] = json!("other");
        assert!(verify_parent(&target, "drive", "Customer", "notes.html").is_err());
    }
}
