use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct Vault {
    path: PathBuf,
    account: String,
    generation: Arc<Mutex<Option<i64>>>,
}

impl Vault {
    pub fn new(root: &Path, account: &str) -> Self {
        Self {
            path: root.join("workiq-login.db"),
            account: account.to_ascii_lowercase(),
            generation: Arc::new(Mutex::new(None)),
        }
    }

    fn database(&self) -> Result<rusqlite::Connection, String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "Create WorkIQ credential directory failed")?;
        }
        let db = rusqlite::Connection::open(&self.path)
            .map_err(|_| "Open WorkIQ credential vault failed")?;
        db.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "Set credential vault deadline failed")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS login(account TEXT PRIMARY KEY, payload BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS login_generation(account TEXT PRIMARY KEY, generation INTEGER NOT NULL);",
        )
        .map_err(|_| "Initialize credential vault failed")?;
        Ok(db)
    }

    fn check_generation(&self, db: &rusqlite::Connection) -> Result<(), String> {
        let current: i64 = db
            .query_row(
                "SELECT generation FROM login_generation WHERE account=?1",
                [&self.account],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "Read credential generation failed")?
            .unwrap_or(0);
        let mut bound = self
            .generation
            .lock()
            .map_err(|_| "Credential binding is unavailable")?;
        if bound.is_some_and(|generation| generation != current) {
            return Err("This saved sign-in was removed; reconnect WorkIQ".into());
        }
        *bound = Some(current);
        Ok(())
    }

    pub async fn bind(&self) -> Result<(), AuthError> {
        self.run(|vault| vault.check_generation(&vault.database()?))
            .await
    }

    async fn run<T: Send + 'static>(
        &self,
        action: impl FnOnce(Self) -> Result<T, String> + Send + 'static,
    ) -> Result<T, AuthError> {
        let vault = self.clone();
        tokio::task::spawn_blocking(move || action(vault))
            .await
            .map_err(|_| AuthError::CredentialStoreError("Credential worker stopped".into()))?
            .map_err(AuthError::CredentialStoreError)
    }
}

#[async_trait::async_trait]
impl CredentialStore for Vault {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        self.run(|vault| {
            let db = vault.database()?;
            vault.check_generation(&db)?;
            let encrypted: Option<Vec<u8>> = db
                .query_row(
                    "SELECT payload FROM login WHERE account=?1",
                    [&vault.account],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| "Read credential vault failed")?;
            let Some(encrypted) = encrypted else {
                return Ok(None);
            };
            if encrypted.len() > 131_072 {
                return Err("Credential vault record exceeds its bound".into());
            }
            let mut plain = protect(&encrypted, &vault.account, false)?;
            let result: Result<StoredCredentials, _> = serde_json::from_slice(&plain);
            plain.fill(0);
            let credentials = result.map_err(|_| "Credential vault record is invalid")?;
            if credentials.client_id != super::hosted::CLIENT_ID
                || credentials.issuer.as_deref()
                    != Some(&format!(
                        "https://login.microsoftonline.com/{}/v2.0",
                        super::hosted::TENANT
                    ))
            {
                return Err(
                    "Credential vault issuer or app identity changed; reconnect WorkIQ".into(),
                );
            }
            Ok(Some(credentials))
        })
        .await
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        self.run(move |vault| {
            let mut plain =
                serde_json::to_vec(&credentials).map_err(|_| "Encode credentials failed")?;
            if plain.len() > 65_536 {
                plain.fill(0);
                return Err("Credentials exceed the vault bound".into());
            }
            let encrypted = protect(&plain, &vault.account, true);
            plain.fill(0);
            let encrypted = encrypted?;
            let mut db = vault.database()?;
            let transaction = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| "Lock credential vault failed")?;
            vault.check_generation(&transaction)?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO login(account,payload) VALUES(?1,?2)",
                    rusqlite::params![vault.account, encrypted],
                )
                .map_err(|_| "Save credential vault failed")?;
            transaction
                .commit()
                .map_err(|_| "Commit credential vault failed")?;
            Ok(())
        })
        .await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.run(|vault| {
            let mut db = vault.database()?;
            db.execute_batch("PRAGMA secure_delete=ON;")
                .map_err(|_| "Configure credential removal failed")?;
            let transaction = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| "Lock credential removal failed")?;
            transaction.execute("DELETE FROM login WHERE account=?1", [&vault.account])
                .map_err(|_| "Remove stored WorkIQ sign-in failed")?;
            transaction.execute(
                "INSERT INTO login_generation VALUES(?1,1) ON CONFLICT(account) DO UPDATE SET generation=generation+1",
                [&vault.account],
            ).map_err(|_| "Revoke credential refresh failed")?;
            transaction.commit().map_err(|_| "Commit credential removal failed")?;
            Ok(())
        })
        .await
    }
}

fn protect(data: &[u8], account: &str, encrypt: bool) -> Result<Vec<u8>, String> {
    let binding = format!(
        "Tomlook WorkIQ v1:{}:{}:{}",
        super::hosted::TENANT,
        super::hosted::CLIENT_ID,
        account
    );
    crate::credentials::protect(data, &binding, encrypt)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn credentials() -> StoredCredentials {
        serde_json::from_value(serde_json::json!({
            "client_id": super::super::hosted::CLIENT_ID,
            "issuer": format!("https://login.microsoftonline.com/{}/v2.0", super::super::hosted::TENANT),
            "token_response": {
                "access_token": "synthetic-vault-access-token",
                "refresh_token": "synthetic-vault-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600
            },
            "granted_scopes": ["fixture"],
            "token_received_at": 1
        })).unwrap()
    }

    #[tokio::test]
    async fn saved_credentials_survive_reopen_and_clear_blocks_late_refresh() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::new(root.path(), "fixture@microsoft.com");
        vault.bind().await.unwrap();
        vault.save(credentials()).await.unwrap();
        let reopened = Vault::new(root.path(), "FIXTURE@microsoft.com");
        let saved = reopened.load().await.unwrap().unwrap();
        assert!(saved.token_response.is_some());
        let db = vault.database().unwrap();
        let payload: Vec<u8> = db
            .query_row("SELECT payload FROM login", [], |row| row.get(0))
            .unwrap();
        assert!(!payload.windows(6).any(|window| window == b"token\""));
        drop(db);
        reopened.clear().await.unwrap();
        assert!(vault.save(credentials()).await.is_err());
        let fresh = Vault::new(root.path(), "fixture@microsoft.com");
        assert!(fresh.load().await.unwrap().is_none());
        fresh.save(credentials()).await.unwrap();
        assert!(fresh.load().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn damaged_cross_account_and_wrong_identity_records_are_not_restored() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::new(root.path(), "fixture@microsoft.com");
        vault.save(credentials()).await.unwrap();
        let other = Vault::new(root.path(), "other@microsoft.com");
        assert!(other.load().await.unwrap().is_none());
        let db = vault.database().unwrap();
        db.execute(
            "INSERT INTO login SELECT 'other@microsoft.com',payload FROM login WHERE account='fixture@microsoft.com'",
            [],
        ).unwrap();
        assert!(other.load().await.is_err());
        db.execute(
            "UPDATE login SET payload=X'000102' WHERE account='fixture@microsoft.com'",
            [],
        )
        .unwrap();
        assert!(vault.load().await.is_err());
        let mut wrong = credentials();
        wrong.client_id = "wrong-client".into();
        vault.save(wrong).await.unwrap();
        assert!(vault.load().await.is_err());
        let mut wrong = credentials();
        wrong.issuer = Some("https://other.example/v2.0".into());
        vault.save(wrong).await.unwrap();
        assert!(vault.load().await.is_err());
    }
    #[test]
    fn encrypted_sign_in_is_user_and_account_bound() {
        let input = b"synthetic credential, never a real token";
        let encrypted = protect(input, "fixture@microsoft.com", true).unwrap();
        assert!(!encrypted.windows(input.len()).any(|window| window == input));
        assert_eq!(
            protect(&encrypted, "fixture@microsoft.com", false).unwrap(),
            input
        );
        assert!(protect(&encrypted, "different@microsoft.com", false).is_err());
    }
}
