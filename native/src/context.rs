use crate::evidence::Recorder;
use async_trait::async_trait;
use github_copilot_sdk::{Tool, ToolInvocation, ToolResult, ToolResultExpanded, tool::ToolHandler};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
    time::{Duration, Instant},
};

pub const SEARCH: &str = "tomlook_local_search";
pub const READ: &str = "tomlook_local_read";

pub(crate) fn read_gate() -> Arc<tokio::sync::Semaphore> {
    static GATE: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    root: usize,
    path: String,
    #[serde(default)]
    offset: usize,
}

pub fn valid_arguments(name: &str, arguments: &Value) -> bool {
    match name {
        SEARCH => serde_json::from_value::<SearchArgs>(arguments.clone()).is_ok_and(|args| {
            !args.query.trim().is_empty()
                && args.query.len() <= 200
                && !args.query.chars().any(char::is_control)
        }),
        READ => serde_json::from_value::<ReadArgs>(arguments.clone())
            .is_ok_and(|args| safe_relative(&args.path) && args.offset <= 1_048_576),
        _ => false,
    }
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 2000
        && !path.chars().any(char::is_control)
        && Path::new(path)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        && !path.contains(':')
}

pub(crate) fn approved_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute()
        || root
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        return Err("Choose a specific local context folder, not an entire drive".into());
    }
    #[cfg(windows)]
    if !matches!(root.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("Context folders must be on a local drive".into());
    }
    verify_components(root)?;
    let root = root
        .canonicalize()
        .map_err(|_| "A context folder is unavailable; check Insights activity")?;
    #[cfg(windows)]
    if !matches!(root.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("Context folders must resolve to a local drive".into());
    }
    if !root.is_dir() {
        return Err("A context folder is not a directory".into());
    }
    Ok(root)
}

pub(crate) fn blocked(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        name.starts_with('.')
            || matches!(
                name.as_str(),
                "node_modules"
                    | "target"
                    | "credentials"
                    | "secrets"
                    | "vendor"
                    | "dist"
                    | "build"
                    | "venv"
                    | "__pycache__"
                    | "packages"
            )
            || name.contains("token")
            || name.contains("credential")
            || name.contains("apikey")
            || name.contains("password")
            || name.contains("secret")
    })
}

pub(crate) fn link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn hidden_or_system(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x6 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

pub(crate) fn text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "txt"
                    | "rs"
                    | "py"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "cs"
                    | "html"
                    | "json"
                    | "yaml"
                    | "yml"
            )
        })
}

fn search(roots: &[PathBuf], query: &str) -> Result<Value, String> {
    let query = query.to_lowercase();
    let started = Instant::now();
    let mut found = Vec::new();
    let mut visited = 0;
    let mut truncated = false;
    for (root_id, root) in roots.iter().enumerate() {
        let root = approved_root(root)?;
        let mut pending = VecDeque::from([(root.clone(), 0)]);
        while let Some((directory, depth)) = pending.pop_front() {
            let entries = std::fs::read_dir(&directory)
                .map_err(|_| "A context folder could not be searched")?;
            for entry in entries {
                visited += 1;
                if visited > 10_000
                    || started.elapsed() > Duration::from_secs(3)
                    || found.len() >= 40
                {
                    truncated = true;
                    break;
                }
                let path = entry
                    .map_err(|_| "A context folder entry could not be read")?
                    .path();
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|_| "Context path escaped its folder")?;
                if blocked(relative) {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|_| "A context file could not be inspected")?;
                if link(&metadata) || hidden_or_system(&metadata) {
                    continue;
                }
                if metadata.is_dir() && depth < 5 {
                    pending.push_back((path, depth + 1));
                } else if metadata.is_file()
                    && relative.to_string_lossy().to_lowercase().contains(&query)
                {
                    found.push(json!({"root": root_id, "path": relative.to_string_lossy(), "text_readable": text_file(relative)}));
                }
            }
            if truncated {
                break;
            }
        }
        if truncated {
            break;
        }
    }
    Ok(json!({"matches": found, "truncated": truncated}))
}

pub(crate) fn verify_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if matches!(part, std::path::Component::Prefix(_)) {
            continue;
        }
        let metadata =
            std::fs::symlink_metadata(&current).map_err(|_| "A source path is unavailable")?;
        if link(&metadata) {
            return Err("Local sources do not follow links, junctions or cloud placeholders; use WorkIQ for cloud documents".into());
        }
    }
    Ok(())
}

fn read(roots: &[PathBuf], args: ReadArgs) -> Result<Value, String> {
    let root = roots.get(args.root).ok_or("Unknown context folder")?;
    let content = read_document(root, &args.path)?;
    document_chunk(
        args.root as i64,
        &args.path,
        &root.display().to_string(),
        content,
        args.offset,
    )
}

pub(crate) fn document_chunk(
    root: i64,
    path: &str,
    folder: &str,
    content: String,
    offset: usize,
) -> Result<Value, String> {
    if offset > content.len() || !content.is_char_boundary(offset) {
        return Err("Invalid document offset; use the previous next_offset".into());
    }
    let end = content.floor_char_boundary((offset + 8_000).min(content.len()));
    Ok(
        json!({"root":root,"folder":folder,"path":path,"content":&content[offset..end],
        "offset":offset,"next_offset": if end < content.len() { Some(end) } else { None },
        "document_bytes":content.len()}),
    )
}

pub(crate) fn read_document(root: &Path, relative: &str) -> Result<String, String> {
    if !safe_relative(relative) || blocked(Path::new(relative)) || !text_file(Path::new(relative)) {
        return Err("Only nonsensitive text files inside the chosen folders can be read".into());
    }
    let root = approved_root(root)?;
    let mut path = root.clone();
    for part in Path::new(relative).components() {
        path.push(part.as_os_str());
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|_| "Context file is unavailable")?;
        if link(&metadata) {
            return Err("Context reads do not follow links or junctions".into());
        }
        if hidden_or_system(&metadata) {
            return Err("Context reads do not include hidden or system files".into());
        }
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Context file is unavailable")?;
    if !canonical.starts_with(&root) {
        return Err("Context file escaped its chosen folder".into());
    }
    use std::io::Read;
    let mut file =
        std::fs::File::open(&canonical).map_err(|_| "Context file could not be opened")?;
    #[cfg(windows)]
    verify_open_file(&file, &root)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| "Context file could not be read")?;
    if bytes.len() > 1_048_576 {
        return Err("Context file exceeds the 1 MiB raw-document limit".into());
    }

    #[cfg(windows)]
    fn verify_open_file(file: &std::fs::File, root: &Path) -> Result<(), String> {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFinalPathNameByHandleW(
                handle: *mut std::ffi::c_void,
                path: *mut u16,
                size: u32,
                flags: u32,
            ) -> u32;
        }
        let mut buffer = vec![0_u16; 32_768];
        // Verify the opened handle, not a path that another process could replace.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if length == 0 || length >= buffer.len() {
            return Err("The opened context file's location could not be verified".into());
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
        if !path.starts_with(root) {
            return Err("The opened context file escaped its chosen folder".into());
        }
        Ok(())
    }
    let content = String::from_utf8(bytes).map_err(|_| "Context file is not UTF-8 text")?;
    if credential_fields(&content) {
        return Err(
            "Context file contains credential fields; choose a nonsensitive document".into(),
        );
    }
    if Path::new(relative)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
    {
        Ok(crate::html_text::extract(&content))
    } else {
        Ok(content)
    }
}

pub(crate) fn credential_fields(content: &str) -> bool {
    content.contains("PRIVATE KEY-----")
        || ["ghp_", "github_pat_", "glpat-"].iter().any(|prefix| {
            content.match_indices(prefix).any(|(position, _)| {
                content[position + prefix.len()..]
                    .chars()
                    .take_while(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                    })
                    .count()
                    >= 30
            })
        })
        || content.lines().any(|line| {
            line.split_once('=')
                .or_else(|| line.split_once(':'))
                .is_some_and(|(name, value)| {
                    let name = name
                        .trim()
                        .trim_matches(['"', '\''])
                        .to_ascii_lowercase()
                        .replace(['_', '-'], "");
                    !value.trim().is_empty()
                        && [
                            "password",
                            "passwd",
                            "accesstoken",
                            "refreshtoken",
                            "apikey",
                            "clientsecret",
                            "privatekey",
                            "authorization",
                        ]
                        .iter()
                        .any(|key| name.ends_with(key))
                })
        })
}

struct LocalRead {
    roots: Arc<Vec<PathBuf>>,
    index: Option<Arc<crate::source_index::Index>>,
    remaining: Arc<AtomicUsize>,
    recorder: Arc<Recorder>,
    chat: Option<Arc<crate::chat::Run>>,
}

#[async_trait]
impl ToolHandler for LocalRead {
    async fn call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolResult, github_copilot_sdk::Error> {
        let name = invocation.tool_name.clone();
        let result = async {
            if !valid_arguments(&name, &invocation.arguments) {
                return Err("Invalid local context read".into());
            }
            crate::copilot::reserve_workiq_read(&self.remaining)
                .map_err(|_| "This answer reached its local-read budget")?;
            let permit = read_gate()
                .acquire_owned()
                .await
                .map_err(|_| "Local read admission stopped")?;
            if self
                .chat
                .as_ref()
                .is_some_and(|run| run.cancelled.load(std::sync::atomic::Ordering::Acquire))
            {
                return Err("This question was stopped; no further read was sent".into());
            }
            let roots = self.roots.clone();
            let index = self.index.clone();
            tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    if name == SEARCH {
                        let args: SearchArgs = serde_json::from_value(invocation.arguments)
                            .map_err(|_| "Invalid context search")?;
                        index.map_or_else(
                            || search(&roots, &args.query),
                            |index| index.search(&args.query),
                        )
                    } else {
                        let args: ReadArgs = serde_json::from_value(invocation.arguments)
                            .map_err(|_| "Invalid context file")?;
                        if let Some(index) = index {
                            index.read(args.root, &args.path, args.offset)
                        } else {
                            read(&roots, args)
                        }
                    }
                }),
            )
            .await
            .map_err(|_| "Local context read timed out")?
            .map_err(|_| "Local context worker stopped")?
        }
        .await;
        if let (Some(run), Ok(value)) = (&self.chat, &result) {
            run.protect_result(value);
        }
        Ok(result_for(&self.recorder, &invocation.tool_name, result))
    }
}

pub(crate) fn result_for(
    recorder: &Recorder,
    name: &str,
    result: Result<Value, String>,
) -> ToolResult {
    let result = result.and_then(|value| {
        let text = serde_json::to_string(&value).map_err(|_| "Encode tool result failed")?;
        let text = recorder.redact(&text);
        if text.len() > 64_000 {
            return Err("Tool result exceeds its 64,000-byte limit; narrow the query".into());
        }
        Ok((value, text))
    });
    let (text, kind) = match result {
        Ok((value, text)) => {
            recorder.record(name, &value, true);
            (text, "success")
        }
        Err(error) => {
            recorder.record(name, &Value::String(error.clone()), false);
            (recorder.redact(&error), "failure")
        }
    };
    ToolResult::Expanded(ToolResultExpanded::new(text, kind))
}

pub fn tools(roots: Vec<PathBuf>, recorder: Arc<Recorder>) -> Vec<Tool> {
    source_tools(roots, None, recorder, 8)
}

pub fn indexed_tools(
    index: Arc<crate::source_index::Index>,
    recorder: Arc<Recorder>,
    prefetched: bool,
) -> Vec<Tool> {
    source_tools(
        Vec::new(),
        Some(index),
        recorder,
        if prefetched { 7 } else { 8 },
    )
}

pub fn indexed_chat_tools(
    index: Arc<crate::source_index::Index>,
    recorder: Arc<Recorder>,
    run: Arc<crate::chat::Run>,
) -> Vec<Tool> {
    read_tools(Arc::new(LocalRead {
        roots: Arc::new(Vec::new()),
        index: Some(index),
        remaining: run.local.clone(),
        recorder,
        chat: Some(run),
    }))
}

fn source_tools(
    roots: Vec<PathBuf>,
    index: Option<Arc<crate::source_index::Index>>,
    recorder: Arc<Recorder>,
    budget: usize,
) -> Vec<Tool> {
    let handler = Arc::new(LocalRead {
        roots: Arc::new(roots),
        index,
        remaining: Arc::new(AtomicUsize::new(budget)),
        recorder,
        chat: None,
    });
    read_tools(handler)
}

fn read_tools(handler: Arc<LocalRead>) -> Vec<Tool> {
    vec![
        Tool::new(SEARCH).with_description("Search paths and indexed document text in authorized repository collections and local folders. Use specific topic/customer words. The result reports freshness, limits and partial coverage. New repositories are discovered automatically. Never send private filenames or contents to public web.")
            .with_parameters(json!({"type":"object","properties":{"query":{"type":"string","maxLength":200}},"required":["query"],"additionalProperties":false})).with_handler(handler.clone()),
        Tool::new(READ).with_description("Read a bounded text chunk from a returned root/path. HTML slides are converted to readable text without executing scripts. Continue with next_offset if needed. Raw files are limited to 1 MiB. Cite the returned folder and path, never invent public URLs. For cloud-only OneDrive documents, use WorkIQ.")
            .with_parameters(json!({"type":"object","properties":{"root":{"type":"integer","minimum":0},"path":{"type":"string","maxLength":2000},"offset":{"type":"integer","minimum":0,"maximum":1048576}},"required":["root","path"],"additionalProperties":false})).with_handler(handler),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_html_is_extracted_and_utf8_documents_are_chunked_without_loss() {
        let root = tempfile::tempdir().unwrap();
        let html = format!(
            "<style>{}</style><h1>Useful demo</h1><p>Customer next step</p>",
            "css".repeat(20_000)
        );
        std::fs::write(root.path().join("slides.html"), html).unwrap();
        let content = read_document(root.path(), "slides.html").unwrap();
        assert!(content.contains("Useful demo"));
        assert!(content.contains("Customer next step"));
        assert!(content.len() < 100);
        let text = "\u{010d}".repeat(10_000);
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let chunk = document_chunk(0, "demo.md", "fixture", text.clone(), offset).unwrap();
            restored.push_str(chunk["content"].as_str().unwrap());
            if let Some(next) = chunk["next_offset"].as_u64() {
                offset = next as usize;
            } else {
                break;
            }
        }
        assert_eq!(restored, text);
        assert!(document_chunk(0, "demo.md", "fixture", text, 1).is_err());
    }

    #[test]
    fn context_reads_are_scoped_bounded_and_skip_sensitive_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("demo.md"), "Fixture demo").unwrap();
        std::fs::write(root.path().join("token-demo.md"), "Must not read").unwrap();
        let roots = vec![root.path().to_owned()];
        let found = search(&roots, "demo").unwrap();
        assert_eq!(found["matches"].as_array().unwrap().len(), 1);
        assert_eq!(
            read(
                &roots,
                ReadArgs {
                    root: 0,
                    path: "demo.md".into(),
                    offset: 0,
                }
            )
            .unwrap()["content"],
            "Fixture demo"
        );
        for path in [
            "..\\demo.md",
            "D:\\demo.md",
            "token-demo.md",
            ".env",
            "demo.pptx",
        ] {
            assert!(
                read(
                    &roots,
                    ReadArgs {
                        root: 0,
                        path: path.into(),
                        offset: 0,
                    }
                )
                .is_err()
            );
        }
        std::fs::write(root.path().join("large.txt"), "a".repeat(1_048_577)).unwrap();
        assert!(
            read(
                &roots,
                ReadArgs {
                    root: 0,
                    path: "large.txt".into(),
                    offset: 0,
                }
            )
            .is_err()
        );
    }
}
