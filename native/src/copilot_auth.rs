use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
use tokio::sync::watch;

// Public OAuth application shipped by Copilot CLI; this is not a client secret.
const CLIENT_ID: &str = "Ov23ctDVkRmgkPke0Mmm";
const SCOPE: &str = "read:user read:org";
const BINDING: &str = "Tomlook Copilot v1:https://github.com:Ov23ctDVkRmgkPke0Mmm";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    token: String,
    login: String,
    expires_at: Option<i64>,
}

impl Saved {
    fn validate(&self) -> Result<(), String> {
        Self::validate_token(&self.token)?;
        if self.login.is_empty()
            || self.login.len() > 100
            || !self
                .login
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("Saved Copilot sign-in is invalid; sign in again".into());
        }
        if self
            .expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp() + 60)
        {
            return Err("Saved Copilot sign-in expired; sign in again".into());
        }
        Ok(())
    }

    fn validate_token(token: &str) -> Result<(), String> {
        if !token.starts_with("gho_")
            || !(5..=8000).contains(&token.len())
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("Copilot sign-in returned an invalid OAuth token; sign in again".into());
        }
        Ok(())
    }

    pub(crate) fn into_token(self) -> String {
        self.token
    }
}

fn database(root: &Path) -> Result<rusqlite::Connection, String> {
    std::fs::create_dir_all(root).map_err(|_| "Create Copilot credential directory failed")?;
    let db = rusqlite::Connection::open(root.join("copilot-login.db"))
        .map_err(|_| "Open Copilot credentials failed")?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|_| "Set Copilot credential deadline failed")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS credentials(id INTEGER PRIMARY KEY CHECK(id=1),payload BLOB NOT NULL)")
        .map_err(|_| "Initialize Copilot credentials failed")?;
    Ok(db)
}

pub(crate) async fn load(root: &Path) -> Result<Option<Saved>, String> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if !root
            .join("copilot-login.db")
            .try_exists()
            .map_err(|_| "Inspect saved Copilot sign-in failed")?
        {
            return Ok(None);
        }
        let db = database(&root)?;
        let encrypted: Option<Vec<u8>> = db
            .query_row("SELECT payload FROM credentials WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|_| "Read Copilot credentials failed")?;
        let Some(encrypted) = encrypted else {
            return Ok(None);
        };
        if encrypted.len() > 32_768 {
            return Err("Saved Copilot credentials exceed their bound".into());
        }
        let mut plain = crate::credentials::protect(&encrypted, BINDING, false)?;
        let decoded = serde_json::from_slice::<Saved>(&plain);
        plain.fill(0);
        let saved = decoded.map_err(|_| "Saved Copilot credentials are invalid; sign in again")?;
        saved.validate()?;
        Ok(Some(saved))
    })
    .await
    .map_err(|_| "Copilot credential worker stopped")?
}

async fn save(
    root: &Path,
    saved: Saved,
    cancelled: watch::Receiver<bool>,
    shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        saved.validate()?;
        let mut plain =
            serde_json::to_vec(&saved).map_err(|_| "Encode Copilot credentials failed")?;
        let encrypted = crate::credentials::protect(&plain, BINDING, true);
        plain.fill(0);
        let encrypted = encrypted?;
        let mut db = database(&root)?;
        let transaction = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "Lock Copilot credentials failed")?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO credentials VALUES(1,?1)",
                [encrypted],
            )
            .map_err(|_| "Save Copilot credentials failed")?;
        if *cancelled.borrow() || *shutdown.borrow() {
            return Err("AI device sign-in cancelled before saving".into());
        }
        transaction
            .commit()
            .map_err(|_| "Commit Copilot credentials failed".into())
    })
    .await
    .map_err(|_| "Copilot credential worker stopped")?
}

pub(crate) async fn sign_in(
    root: &Path,
    cancelled: watch::Receiver<bool>,
    shutdown: watch::Receiver<bool>,
    progress: impl Fn(String),
) -> Result<(), String> {
    let authenticate = async {
        let mut token =
            crate::connectors::github_device_token(CLIENT_ID, Some(SCOPE), false, progress).await?;
        if token.error.is_some() || token.token_type.as_deref() != Some("bearer") {
            return Err("Copilot device sign-in returned an unsupported token".to_string());
        }
        let expires_at = match token.expires_in {
            Some(seconds) if (1..=31_536_000).contains(&seconds) => {
                Some(chrono::Utc::now().timestamp() + seconds as i64)
            }
            Some(_) => {
                return Err("Copilot device sign-in returned an invalid token lifetime".into());
            }
            None => None,
        };
        let token = token
            .access_token
            .take()
            .ok_or("Copilot device sign-in returned no token")?;
        Saved::validate_token(&token)?;
        let login = crate::connectors::github_identity(&token).await?;
        let saved = Saved {
            token,
            login,
            expires_at,
        };
        saved.validate()?;
        Ok(saved)
    };
    sign_in_with(root, cancelled, shutdown, authenticate).await
}

async fn sign_in_with(
    root: &Path,
    mut cancelled: watch::Receiver<bool>,
    mut shutdown: watch::Receiver<bool>,
    authenticate: impl std::future::Future<Output = Result<Saved, String>>,
) -> Result<(), String> {
    if *cancelled.borrow() || *shutdown.borrow() {
        return Err("AI device sign-in cancelled before authentication".into());
    }
    let saved = tokio::select! {
        biased;
        _ = cancelled.changed() => return Err("AI device sign-in cancelled".into()),
        _ = shutdown.changed() => return Err("AI device sign-in stopped during shutdown".into()),
        result = tokio::time::timeout(Duration::from_secs(900), authenticate) =>
            result.map_err(|_| "AI device sign-in expired; sign in again")??,
    };
    save(root, saved, cancelled, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Saved {
        Saved {
            token: "gho_fixtureOnlyNotARealToken".into(),
            login: "fixture-user".into(),
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn missing_login_never_creates_a_database_or_imports_legacy_runtime_tokens() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("copilot")).unwrap();
        std::fs::write(
            root.path().join("copilot").join("config.json"),
            r#"{"oauth_token":"gho_legacy"}"#,
        )
        .unwrap();
        assert!(load(root.path()).await.unwrap().is_none());
        assert!(!root.path().join("copilot-login.db").exists());
    }

    #[tokio::test]
    async fn already_cancelled_login_never_contacts_github_or_creates_storage() {
        for shutdown_first in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (_cancel, cancelled) = watch::channel(!shutdown_first);
            let (_stop, shutdown) = watch::channel(shutdown_first);
            assert!(
                sign_in(root.path(), cancelled, shutdown, |_| panic!(
                    "No authorization request"
                ))
                .await
                .is_err()
            );
            assert!(!root.path().join("copilot-login.db").exists());
        }
    }

    #[tokio::test]
    async fn cancellation_and_shutdown_interrupt_pending_authorization_without_saving() {
        for shutdown_first in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (cancel, cancelled) = watch::channel(false);
            let (stop, shutdown) = watch::channel(false);
            let authorize = async move {
                if shutdown_first {
                    stop.send(true).unwrap();
                } else {
                    cancel.send(true).unwrap();
                }
                std::future::pending::<Result<Saved, String>>().await
            };
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                sign_in_with(root.path(), cancelled, shutdown, authorize),
            )
            .await
            .unwrap();
            assert!(result.unwrap_err().contains(if shutdown_first {
                "shutdown"
            } else {
                "cancelled"
            }));
            assert!(!root.path().join("copilot-login.db").exists());
        }
    }

    #[tokio::test]
    async fn rejected_authorization_never_creates_a_saved_sign_in() {
        let root = tempfile::tempdir().unwrap();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        assert_eq!(
            sign_in_with(root.path(), cancelled, shutdown, async {
                Err("GitHub sign-in was declined".into())
            })
            .await
            .unwrap_err(),
            "GitHub sign-in was declined"
        );
        assert!(!root.path().join("copilot-login.db").exists());
    }

    #[test]
    fn saved_identity_rejects_unsupported_credentials_and_expiry() {
        for value in [
            "",
            "ghp_classic",
            "github_pat_fixture",
            "gho_has whitespace",
            "gho_bad\nvalue",
        ] {
            let mut record = saved();
            record.token = value.into();
            assert!(record.validate().is_err());
        }
        let mut record = saved();
        record.expires_at = Some(chrono::Utc::now().timestamp());
        assert!(record.validate().is_err());
        record.expires_at = Some(chrono::Utc::now().timestamp() + 3600);
        assert!(record.validate().is_ok());
        record.login = "not/an/account".into();
        assert!(record.validate().is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn vault_roundtrip_is_encrypted_and_rejects_tampering_and_other_bindings() {
        let root = tempfile::tempdir().unwrap();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        save(root.path(), saved(), cancelled, shutdown)
            .await
            .unwrap();
        let bytes = std::fs::read(root.path().join("copilot-login.db")).unwrap();
        assert!(
            !bytes
                .windows(saved().token.len())
                .any(|part| part == saved().token.as_bytes())
        );
        let restored = load(root.path()).await.unwrap().unwrap();
        assert_eq!(restored.login, "fixture-user");
        assert_eq!(restored.into_token(), saved().token);
        let db = database(root.path()).unwrap();
        let encrypted: Vec<u8> = db
            .query_row("SELECT payload FROM credentials", [], |row| row.get(0))
            .unwrap();
        assert!(crate::credentials::protect(&encrypted, "another-provider", false).is_err());
        let mut corrupted = encrypted;
        corrupted[0] ^= 0xff;
        db.execute("UPDATE credentials SET payload=?1", [corrupted])
            .unwrap();
        assert!(load(root.path()).await.is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelled_replacement_preserves_the_previous_encrypted_sign_in() {
        let root = tempfile::tempdir().unwrap();
        let (_cancel, cancelled) = watch::channel(false);
        let (_stop, shutdown) = watch::channel(false);
        save(root.path(), saved(), cancelled, shutdown.clone())
            .await
            .unwrap();
        let (_cancel, cancelled) = watch::channel(true);
        let mut replacement = saved();
        replacement.token = "gho_replacement".into();
        assert!(
            save(root.path(), replacement, cancelled, shutdown)
                .await
                .is_err()
        );
        assert_eq!(
            load(root.path()).await.unwrap().unwrap().into_token(),
            saved().token
        );
    }
}
