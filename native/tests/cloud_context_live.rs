use std::{fs::OpenOptions, io::Write, path::PathBuf, sync::Arc, time::Instant};
use tomlook::{
    evidence::{Recorder, Redactor},
    workiq::connection::Connection,
};

#[tokio::test]
#[ignore = "Requires explicitly authorized saved login, verified cloud folder URL, a private content marker and new private output; performs one real read-only context request"]
async fn saved_login_retrieves_verified_cloud_folder_content() {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").expect("Set TOMLOOK_LIVE_INSIGHTS_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CLOUD_OUTPUT").expect("Set TOMLOOK_LIVE_CLOUD_OUTPUT"),
    );
    let account =
        std::env::var("TOMLOOK_LIVE_WORK_ACCOUNT").expect("Set TOMLOOK_LIVE_WORK_ACCOUNT");
    let folder =
        std::env::var("TOMLOOK_LIVE_CLOUD_FOLDER_URL").expect("Set TOMLOOK_LIVE_CLOUD_FOLDER_URL");
    let marker = std::env::var("TOMLOOK_LIVE_CLOUD_CONTENT_MARKER")
        .expect("Set TOMLOOK_LIVE_CLOUD_CONTENT_MARKER from a separately verified original file");
    let file_url =
        std::env::var("TOMLOOK_LIVE_CLOUD_FILE_URL").expect("Set TOMLOOK_LIVE_CLOUD_FILE_URL");
    assert!(root.is_absolute() && output.is_absolute());
    assert!(!output.exists(), "Never overwrite earlier evidence");
    assert!(!marker.trim().is_empty() && marker.len() <= 500);
    let url = url::Url::parse(&folder).unwrap();
    assert_eq!(url.scheme(), "https");
    assert!(url.username().is_empty() && url.password().is_none());
    assert!(url.query().is_none() && url.fragment().is_none());
    assert!(url.host_str().unwrap().ends_with("-my.sharepoint.com"));
    assert!(url.path().starts_with("/personal/") && !url.path().ends_with('/'));
    assert!(file_url.starts_with(&format!("{folder}/")));
    assert!(tomlook::evidence::safe_source_url(&file_url));
    let query = format!(
        "Search only inside this verified OneDrive folder URL (data): {folder}. \
         Find existing HTML presentations about agentic SDLC and loop engineering. \
         Return original file URLs, exact titles, authors and one exact slide heading \
         from readable file content. Explicitly distinguish metadata-only results \
         from actual readable content; do not infer content from filenames."
    );
    let started = Instant::now();
    let connection = Connection::connect_saved(&root, &account, false, |_| {})
        .await
        .unwrap();
    let result = connection.context(&query).await;
    let closed = connection.close().await;
    let recorder = Recorder::new(Arc::new(Redactor::default()));
    let value = match result {
        Ok(value) => {
            recorder.record(tomlook::workiq::connection::CONTEXT_TOOL, &value, true);
            value
        }

        Err(error) => {
            recorder.record(
                tomlook::workiq::connection::CONTEXT_TOOL,
                &serde_json::Value::String(error.clone()),
                false,
            );
            serde_json::json!({"error":error})
        }
    };
    let evidence = recorder.snapshot().unwrap();
    let original_link_inside_folder = evidence.records.iter().any(|record| {
        record.succeeded
            && record
                .links
                .iter()
                .any(|link| link.starts_with(&format!("{folder}/")))
    }) || value["answer"]
        .as_str()
        .is_some_and(|answer| answer.contains(&file_url));
    let expected_content_marker_observed = value["answer"]
        .as_str()
        .is_some_and(|answer| answer.contains(&marker));
    let artifact = serde_json::json!({
        "diagnostic": "verified_cloud_folder_context",
        "created_at": chrono::Utc::now().to_rfc3339(),
        "elapsed_ms": started.elapsed().as_millis(),
        "verified_folder_url": folder,
        "context": value,
        "retrieved_evidence": &*evidence,
        "original_link_inside_folder": original_link_inside_folder,
        "expected_content_marker_observed": expected_content_marker_observed,
        "owned_connection_cleanup": if closed.is_ok() { "passed" } else { "failed" },
        "not_a_completed_product_insight": true,
        "product_calendar_ledger_and_source_index_modified": false,
    });
    let bytes = serde_json::to_vec_pretty(&artifact).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    let reopened: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(artifact, reopened);
    closed.unwrap();
    assert!(
        original_link_inside_folder && expected_content_marker_observed,
        "Must retrieve an original in-folder link and independently known content, not merely return a usable answer"
    );
}

#[tokio::test]
#[ignore = "Requires authorized app-owned saved login, independently verified private folder/file identities and new private output; performs bounded direct metadata and text reads"]
async fn saved_login_reads_original_cloud_file_without_semantic_search() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tomlook::workiq::connection::cloud::{Arguments, SDK_TOOL};
    let root = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").unwrap());
    let output = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_CLOUD_OUTPUT").unwrap());
    let account = std::env::var("TOMLOOK_LIVE_WORK_ACCOUNT").unwrap();
    let folder = std::env::var("TOMLOOK_LIVE_CLOUD_FOLDER_URL").unwrap();
    let file_url = std::env::var("TOMLOOK_LIVE_CLOUD_FILE_URL").unwrap();
    let path = std::env::var("TOMLOOK_LIVE_CLOUD_FILE_PATH").unwrap();
    let marker = std::env::var("TOMLOOK_LIVE_CLOUD_CONTENT_MARKER").unwrap();
    let expected_children: usize = std::env::var("TOMLOOK_LIVE_CLOUD_CHILD_COUNT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    assert!(!marker.trim().is_empty() && marker.len() <= 500);
    assert!(file_url.starts_with(&format!("{folder}/")));
    assert!(tomlook::evidence::safe_source_url(&file_url));
    let parent = path
        .rsplit_once('/')
        .expect("Use a verified nested file")
        .0
        .to_owned();
    let started = Instant::now();
    let connection = Connection::connect_saved(&root, &account, false, |_| {})
        .await
        .unwrap();
    let remaining = AtomicUsize::new(8);
    let recorder = Recorder::new(Arc::new(Redactor::default()));
    let result = async {
        let listing = connection.cloud_read(&folder, Arguments {path:String::new(),offset:0}, &remaining).await?;
        recorder.record(SDK_TOOL, &listing, true);
        if listing["children"].as_array().map(Vec::len) != Some(expected_children)
            || listing["direct_children_complete"] != true
        {
            return Err("Direct-child catalog did not match independently verified complete folder coverage".to_owned());
        }
        let files = connection.cloud_read(&folder, Arguments {path:parent,offset:0}, &remaining).await?;
        recorder.record(SDK_TOOL, &files, true);
        if !files["children"].as_array().is_some_and(|rows| rows.iter().any(|row| row["path"] == path && row["url"] == file_url && row["readable_text"] == true)) {
            return Err("The verified original readable file was not in the direct native listing".into());
        }
        let document = connection.cloud_read(&folder, Arguments {path,offset:0}, &remaining).await?;
        recorder.record(SDK_TOOL, &document, true);
        if document["_cloud_source"]["url"] != file_url
            || !document["content"].as_str().is_some_and(|text| text.contains(&marker))
        {
            return Err("Original native file content did not match the independently read document".into());
        }
        Ok(serde_json::json!({"listing":listing,"files":files,"document":document}))
    }.await;
    let closed = connection.close().await;
    let value = match &result {
        Ok(value) => value.clone(),
        Err(error) => {
            recorder.record(SDK_TOOL, &serde_json::Value::String(error.clone()), false);
            serde_json::json!({"error":error})
        }
    };
    let artifact = serde_json::json!({
        "diagnostic":"direct_original_cloud_content",
        "created_at":chrono::Utc::now().to_rfc3339(),
        "elapsed_ms":started.elapsed().as_millis(),
        "result":value,
        "verified":result.is_ok(),
        "provider_research_reads":8-remaining.load(Ordering::Acquire),
        "retrieved_evidence": &*recorder.snapshot().unwrap(),
        "owned_connection_cleanup":if closed.is_ok() {"passed"} else {"failed"},
        "not_a_completed_product_insight":true,
        "product_calendar_ledger_and_source_index_modified":false,
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&artifact).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    let reopened: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(artifact, reopened);
    closed.unwrap();
    result.unwrap();
}

#[tokio::test]
#[ignore = "Requires authorized saved login and independently verified private original file URL/path/content marker; proves one direct file read, not directory coverage"]
async fn saved_login_reads_one_verified_original_cloud_file() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tomlook::workiq::connection::cloud::Arguments;
    let root = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_INSIGHTS_ROOT").unwrap());
    let output = PathBuf::from(std::env::var_os("TOMLOOK_LIVE_CLOUD_OUTPUT").unwrap());
    let account = std::env::var("TOMLOOK_LIVE_WORK_ACCOUNT").unwrap();
    let folder = std::env::var("TOMLOOK_LIVE_CLOUD_FOLDER_URL").unwrap();
    let path = std::env::var("TOMLOOK_LIVE_CLOUD_FILE_PATH").unwrap();
    let file_url = std::env::var("TOMLOOK_LIVE_CLOUD_FILE_URL").unwrap();
    let marker = std::env::var("TOMLOOK_LIVE_CLOUD_CONTENT_MARKER").unwrap();
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    assert!(!marker.trim().is_empty() && marker.len() <= 500);
    assert!(file_url.starts_with(&format!("{folder}/")));
    let started = Instant::now();
    let connection = Connection::connect_saved(&root, &account, false, |_| {})
        .await
        .unwrap();
    let remaining = AtomicUsize::new(8);
    let result = connection
        .cloud_read(&folder, Arguments { path, offset: 0 }, &remaining)
        .await;
    let cleanup = connection.close().await;
    let matching = result.as_ref().is_ok_and(|value| {
        value["_cloud_source"]["url"] == file_url
            && value["content"]
                .as_str()
                .is_some_and(|text| text.contains(&marker))
    });
    let artifact = serde_json::json!({
        "diagnostic":"one_original_cloud_file",
        "created_at":chrono::Utc::now().to_rfc3339(),
        "elapsed_ms":started.elapsed().as_millis(),
        "result":match &result {Ok(value)=>value.clone(),Err(error)=>serde_json::json!({"error":error})},
        "original_file_and_independent_content_marker_match":matching,
        "provider_research_reads":8-remaining.load(Ordering::Acquire),
        "owned_connection_cleanup":if cleanup.is_ok() {"passed"} else {"failed"},
        "directory_coverage":"not_established",
        "not_a_completed_product_insight":true,
        "product_calendar_ledger_and_source_index_modified":false,
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&artifact).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    let reopened: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(artifact, reopened);
    cleanup.unwrap();
    result.unwrap();
    assert!(
        matching,
        "Native bytes must match the independently read original document"
    );
}
