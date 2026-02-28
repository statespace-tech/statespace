use crate::args::AppSyncArgs;
use crate::error::Result;
use crate::gateway::GatewayClient;
use crate::gateway::applications::{ApplicationFile, UpsertResult};
use crate::state::{SyncState, load_state, save_state};

pub(crate) trait Upserter {
    fn upsert_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
    ) -> impl std::future::Future<Output = Result<UpsertResult>> + Send;
}

impl Upserter for GatewayClient {
    async fn upsert_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
    ) -> Result<UpsertResult> {
        self.upsert_application(name, files).await
    }
}

pub(crate) async fn run_sync(args: AppSyncArgs, gateway: impl Upserter) -> Result<()> {
    let dir = args.path.canonicalize().map_err(|e| {
        crate::error::Error::cli(format!("Invalid path '{}': {e}", args.path.display()))
    })?;

    let cached = load_state(&dir)?;

    let name = args
        .name
        .or_else(|| cached.as_ref().map(|s| s.name.clone()))
        .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(String::from))
        .ok_or_else(|| crate::error::Error::cli("Could not determine application name"))?;

    let files = GatewayClient::scan_markdown_files(&dir)?;

    if files.is_empty() {
        eprintln!("No .md files found in {}", dir.display());
        return Ok(());
    }

    let checksums: Vec<(String, String)> = files
        .iter()
        .map(|f| (f.path.clone(), f.checksum.clone()))
        .collect();

    if let Some(ref prev) = cached {
        let same_target = prev.name == name;
        if same_target {
            let prev_map: std::collections::HashMap<&str, &str> = prev
                .checksums
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let changed = checksums.len() != prev.checksums.len()
                || checksums
                    .iter()
                    .any(|(p, c)| prev_map.get(p.as_str()) != Some(&c.as_str()));

            if !changed {
                eprintln!("No changes detected, skipping sync.");
                return Ok(());
            }
        }
    }

    eprintln!(
        "Syncing {} file{} to '{name}'...",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );

    let result = gateway.upsert_application(&name, files).await?;

    let action = if result.created { "Created" } else { "Updated" };
    eprintln!("{action} application '{}'", result.name);

    if let Some(ref url) = result.url {
        eprintln!("URL: {url}");
    }

    let state = SyncState::new(result.id, result.name, result.url, result.auth_token)
        .with_checksums(&checksums);

    save_state(&dir, &state)?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type UpsertCall = (String, Vec<ApplicationFile>);
    type RecordedCalls = Arc<Mutex<Vec<UpsertCall>>>;

    struct MockUpserter {
        result: UpsertResult,
        calls: RecordedCalls,
    }

    impl MockUpserter {
        fn new(result: UpsertResult) -> (Self, RecordedCalls) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mock = Self {
                result,
                calls: Arc::clone(&calls),
            };
            (mock, calls)
        }
    }

    impl Upserter for MockUpserter {
        async fn upsert_application(
            &self,
            name: &str,
            files: Vec<ApplicationFile>,
        ) -> Result<UpsertResult> {
            self.calls
                .lock()
                .expect("lock poisoned")
                .push((name.to_string(), files));
            Ok(self.result.clone())
        }
    }

    fn upsert_result(created: bool, name: &str) -> UpsertResult {
        UpsertResult {
            created,
            id: "id-1".to_string(),
            name: name.to_string(),
            url: Some(format!("https://{name}.app.statespace.com")),
            auth_token: None,
        }
    }

    #[tokio::test]
    async fn sync_created_calls_upsert_with_name_and_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("page.md"), "# Hello").expect("write");

        let (mock, calls) = MockUpserter::new(upsert_result(true, "foo"));

        let args = AppSyncArgs {
            path: dir.path().to_path_buf(),
            name: Some("foo".to_string()),
        };

        run_sync(args, mock).await.expect("run_sync");

        let recorded = calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "foo");
        assert_eq!(recorded[0].1.len(), 1);
        assert_eq!(recorded[0].1[0].path, "page.md");
    }

    #[tokio::test]
    async fn sync_updated_persists_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("readme.md"), "# Updated").expect("write");

        let (mock, calls) = MockUpserter::new(upsert_result(false, "bar"));

        let args = AppSyncArgs {
            path: dir.path().to_path_buf(),
            name: Some("bar".to_string()),
        };

        run_sync(args, mock).await.expect("run_sync");

        let recorded = calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "bar");

        // Verify state was persisted
        let state = load_state(&dir.path().canonicalize().expect("canon"))
            .expect("load")
            .expect("state exists");
        assert_eq!(state.name, "bar");
        assert_eq!(state.deployment_id, "id-1");
    }
}
