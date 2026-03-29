use crate::args::AppDeployArgs;
use crate::commands::env::resolve_env_overrides;
use crate::config::load_merged_app_env;
use crate::error::{ApiErrorCode, Error, GatewayError, Result};
use crate::gateway::GatewayClient;
use crate::gateway::applications::{ApplicationFile, DeployResult, UpsertResult, Visibility};
use crate::names::generate_name;
use crate::state::{DeployState, load_state, save_state};
use sha2::{Digest, Sha256};
use statespace_server::initialize_templates;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

const ENV_STATE_CHECKSUM_KEY: &str = "__statespace_env__";

pub(crate) trait DeployGateway {
    fn create_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
        visibility: Option<Visibility>,
    ) -> impl std::future::Future<Output = Result<DeployResult>> + Send;

    fn upsert_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
    ) -> impl std::future::Future<Output = Result<UpsertResult>> + Send;

    fn list_secret_keys(
        &self,
        application_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send;

    fn set_secret(
        &self,
        application_id: &str,
        key: &str,
        value: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn delete_secret(
        &self,
        application_id: &str,
        key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

impl DeployGateway for GatewayClient {
    async fn create_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
        visibility: Option<Visibility>,
    ) -> Result<DeployResult> {
        self.create_application(name, files, visibility).await
    }

    async fn upsert_application(
        &self,
        name: &str,
        files: Vec<ApplicationFile>,
    ) -> Result<UpsertResult> {
        self.upsert_application(name, files).await
    }

    async fn list_secret_keys(&self, application_id: &str) -> Result<Vec<String>> {
        self.list_secret_keys(application_id).await
    }

    async fn set_secret(&self, application_id: &str, key: &str, value: &str) -> Result<()> {
        self.set_secret(application_id, key, value).await
    }

    async fn delete_secret(&self, application_id: &str, key: &str) -> Result<()> {
        self.delete_secret(application_id, key).await
    }
}

#[derive(Debug, Clone)]
struct DeployTarget {
    name: String,
}

#[derive(Debug, Clone)]
struct DeployOutcome {
    created: bool,
    id: String,
    name: String,
    url: Option<String>,
    auth_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SecretSyncSummary {
    upserted: usize,
    deleted: usize,
}

impl DeployOutcome {
    fn from_upsert(result: UpsertResult) -> Self {
        Self {
            created: result.created,
            id: result.id,
            name: result.name,
            url: result.url,
            auth_token: result.auth_token,
        }
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn run_deploy(
    args: AppDeployArgs,
    config_path: &Path,
    gateway: impl DeployGateway,
) -> Result<()> {
    let AppDeployArgs {
        path,
        visibility,
        name,
        env_vars,
        env_file,
        cloud: _,
    } = args;

    match path {
        None => {
            let deploy_env = resolve_env_overrides(
                load_merged_app_env(config_path, None)?,
                &env_vars,
                env_file.as_deref(),
                "deploy",
            )?;

            let name = name.unwrap_or_else(generate_name);
            eprintln!("Creating empty application '{name}'...");

            let result = gateway
                .create_application(&name, Vec::new(), visibility)
                .await
                .map_err(|e| map_create_error(e, &name))?;

            let sync = sync_application_secrets(&gateway, &result.id, &deploy_env).await?;

            eprintln!();
            eprintln!("Created '{name}'");
            eprintln!("  ID: {}", result.id);
            if let Some(ref url) = result.url {
                eprintln!("  URL: {url}");
            }
            if let Some(ref token) = result.auth_token {
                eprintln!("  Token: {token}");
            }
            eprintln!(
                "  Secrets: {} upserted, {} deleted",
                sync.upserted, sync.deleted
            );

            Ok(())
        }
        Some(path) => {
            let dir = path.canonicalize().map_err(|e| {
                crate::error::Error::cli(format!("Invalid path '{}': {e}", path.display()))
            })?;

            if !dir.is_dir() {
                return Err(Error::cli(format!("Not a directory: {}", dir.display())));
            }

            let deploy_env = resolve_env_overrides(
                load_merged_app_env(config_path, Some(&dir))?,
                &env_vars,
                env_file.as_deref(),
                "deploy",
            )?;

            initialize_templates(&dir)
                .await
                .map_err(|e| Error::cli(format!("Failed to initialize templates: {e}")))?;

            if !dir.join("README.md").is_file() {
                return Err(Error::cli(
                    "README.md not found. Create it before deploying your app.".to_string(),
                ));
            }

            let cached = load_state(&dir)?;
            let target = resolve_target(name)?;

            let files = GatewayClient::scan_deploy_files(&dir)?;

            if files.is_empty() {
                eprintln!("No files found in {}", dir.display());
                return Ok(());
            }
            let env_checksum = checksum_env_map(&deploy_env);

            let mut checksums: Vec<(String, String)> = files
                .iter()
                .map(|f| (f.path.clone(), f.checksum.clone()))
                .collect();
            checksums.push((ENV_STATE_CHECKSUM_KEY.to_string(), env_checksum.clone()));

            if let Some(ref prev) = cached {
                let same_target = prev.name == target.name;
                if same_target {
                    let prev_map: HashMap<&str, &str> = prev
                        .checksums
                        .iter()
                        .filter(|(path, _)| path.as_str() != ENV_STATE_CHECKSUM_KEY)
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    let files_changed = checksums
                        .iter()
                        .filter(|(path, _)| path.as_str() != ENV_STATE_CHECKSUM_KEY)
                        .count()
                        != prev_map.len()
                        || checksums
                            .iter()
                            .filter(|(path, _)| path.as_str() != ENV_STATE_CHECKSUM_KEY)
                            .any(|(p, c)| prev_map.get(p.as_str()) != Some(&c.as_str()));
                    let env_changed = prev
                        .checksums
                        .get(ENV_STATE_CHECKSUM_KEY)
                        .map(String::as_str)
                        != Some(env_checksum.as_str());

                    if !files_changed && !env_changed {
                        eprintln!("No changes detected, skipping deploy.");
                        return Ok(());
                    }
                }
            }

            eprintln!(
                "Deploying {} file{} to '{}'...",
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                target.name
            );

            let upsert_result = gateway.upsert_application(&target.name, files).await?;
            let result = DeployOutcome::from_upsert(upsert_result);

            let sync = sync_application_secrets(&gateway, &result.id, &deploy_env).await?;

            let action = if result.created { "Created" } else { "Updated" };
            eprintln!("{action} application '{}'", result.name);

            if let Some(ref url) = result.url {
                eprintln!("URL: {url}");
            }
            eprintln!(
                "Secrets synced: {} upserted, {} deleted",
                sync.upserted, sync.deleted
            );

            let state = DeployState::new(result.id, result.name, result.url, result.auth_token)
                .with_checksums(&checksums);

            save_state(&dir, &state)?;

            Ok(())
        }
    }
}

async fn sync_application_secrets(
    gateway: &impl DeployGateway,
    application_id: &str,
    desired: &HashMap<String, String>,
) -> Result<SecretSyncSummary> {
    let existing_keys = gateway.list_secret_keys(application_id).await?;

    let desired_sorted: BTreeMap<&str, &str> = desired
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    for (key, value) in &desired_sorted {
        gateway.set_secret(application_id, key, value).await?;
    }

    let desired_keys: BTreeSet<&str> = desired_sorted.keys().copied().collect();
    let mut deleted = 0;
    for key in existing_keys {
        if !desired_keys.contains(key.as_str()) {
            gateway.delete_secret(application_id, &key).await?;
            deleted += 1;
        }
    }

    Ok(SecretSyncSummary {
        upserted: desired_sorted.len(),
        deleted,
    })
}

fn checksum_env_map(env: &HashMap<String, String>) -> String {
    let mut entries: Vec<(&str, &str)> = env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    entries.sort_unstable();

    let mut hasher = Sha256::new();
    for (key, value) in entries {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }

    format!("sha256:{:x}", hasher.finalize())
}

fn resolve_target(explicit_name: Option<String>) -> Result<DeployTarget> {
    let Some(name) = explicit_name else {
        return Err(Error::cli(
            "Application name is required for directory deploys. Pass --name <NAME>.".to_string(),
        ));
    };

    Ok(DeployTarget { name })
}

fn map_create_error(error: Error, name: &str) -> Error {
    match error {
        Error::Gateway(GatewayError::Api {
            status: 409,
            code: ApiErrorCode::NameTaken,
            ..
        }) => {
            let mut suggestion = generate_name();
            while suggestion == name {
                suggestion = generate_name();
            }
            Error::cli(format!(
                "Application name '{name}' is already taken. Try `statespace deploy --name {suggestion}`."
            ))
        }
        other => other,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const TEST_CONFIG_PATH: &str = "/tmp/statespace-cli-test-missing-config-c6b80ad3.toml";

    type CreateCall = (String, Vec<ApplicationFile>, Option<Visibility>);
    type UpsertCall = (String, Vec<ApplicationFile>);
    type SetSecretCall = (String, String, String);
    type DeleteSecretCall = (String, String);
    type RecordedCreateCalls = Arc<Mutex<Vec<CreateCall>>>;
    type RecordedUpsertCalls = Arc<Mutex<Vec<UpsertCall>>>;
    type RecordedSetSecretCalls = Arc<Mutex<Vec<SetSecretCall>>>;
    type RecordedDeleteSecretCalls = Arc<Mutex<Vec<DeleteSecretCall>>>;

    struct MockDeployGateway {
        create_result: DeployResult,
        upsert_result: UpsertResult,
        fail_create_with_name_taken: bool,
        existing_secret_keys: Vec<String>,
        create_calls: RecordedCreateCalls,
        upsert_calls: RecordedUpsertCalls,
        set_secret_calls: RecordedSetSecretCalls,
        delete_secret_calls: RecordedDeleteSecretCalls,
    }

    impl MockDeployGateway {
        fn new(
            create_result: DeployResult,
            upsert_result: UpsertResult,
        ) -> (
            Self,
            RecordedCreateCalls,
            RecordedUpsertCalls,
            RecordedSetSecretCalls,
            RecordedDeleteSecretCalls,
        ) {
            let create_calls = Arc::new(Mutex::new(Vec::new()));
            let upsert_calls = Arc::new(Mutex::new(Vec::new()));
            let set_secret_calls = Arc::new(Mutex::new(Vec::new()));
            let delete_secret_calls = Arc::new(Mutex::new(Vec::new()));
            let mock = Self {
                create_result,
                upsert_result,
                fail_create_with_name_taken: false,
                existing_secret_keys: Vec::new(),
                create_calls: Arc::clone(&create_calls),
                upsert_calls: Arc::clone(&upsert_calls),
                set_secret_calls: Arc::clone(&set_secret_calls),
                delete_secret_calls: Arc::clone(&delete_secret_calls),
            };
            (
                mock,
                create_calls,
                upsert_calls,
                set_secret_calls,
                delete_secret_calls,
            )
        }
    }

    impl DeployGateway for MockDeployGateway {
        async fn create_application(
            &self,
            name: &str,
            files: Vec<ApplicationFile>,
            visibility: Option<Visibility>,
        ) -> Result<DeployResult> {
            self.create_calls.lock().expect("lock poisoned").push((
                name.to_string(),
                files,
                visibility,
            ));

            if self.fail_create_with_name_taken {
                return Err(Error::Gateway(GatewayError::Api {
                    status: 409,
                    code: ApiErrorCode::NameTaken,
                    message: format!("Application name '{name}' is already taken"),
                }));
            }

            Ok(self.create_result.clone())
        }

        async fn upsert_application(
            &self,
            name: &str,
            files: Vec<ApplicationFile>,
        ) -> Result<UpsertResult> {
            self.upsert_calls
                .lock()
                .expect("lock poisoned")
                .push((name.to_string(), files));
            Ok(self.upsert_result.clone())
        }

        async fn list_secret_keys(&self, _application_id: &str) -> Result<Vec<String>> {
            Ok(self.existing_secret_keys.clone())
        }

        async fn set_secret(&self, application_id: &str, key: &str, value: &str) -> Result<()> {
            self.set_secret_calls.lock().expect("lock poisoned").push((
                application_id.to_string(),
                key.to_string(),
                value.to_string(),
            ));
            Ok(())
        }

        async fn delete_secret(&self, application_id: &str, key: &str) -> Result<()> {
            self.delete_secret_calls
                .lock()
                .expect("lock poisoned")
                .push((application_id.to_string(), key.to_string()));
            Ok(())
        }
    }

    fn deploy_result(name: &str) -> DeployResult {
        DeployResult {
            id: "id-1".to_string(),
            auth_token: None,
            url: Some(format!("https://{name}.app.statespace.com")),
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

    fn deploy_args(path: Option<std::path::PathBuf>, name: Option<&str>) -> AppDeployArgs {
        AppDeployArgs {
            path,
            visibility: None,
            name: name.map(ToOwned::to_owned),
            env_vars: Vec::new(),
            env_file: None,
        }
    }

    #[tokio::test]
    async fn deploy_without_path_creates_empty_app() {
        let (mock, create_calls, upsert_calls, set_secret_calls, delete_secret_calls) =
            MockDeployGateway::new(deploy_result("test-app"), upsert_result(false, "unused"));

        let args = deploy_args(None, Some("test-app"));

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "test-app");
        assert!(recorded[0].1.is_empty());
        assert!(upsert_calls.lock().expect("lock").is_empty());
        assert!(set_secret_calls.lock().expect("lock").is_empty());
        assert!(delete_secret_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_without_path_uses_random_name() {
        let (mock, create_calls, _, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));

        let args = deploy_args(None, None);

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_ne!(recorded[0].0, "unused");
        assert!(recorded[0].0.contains('-'));
    }

    #[tokio::test]
    async fn deploy_without_path_passes_visibility() {
        let (mock, create_calls, _, _, _) =
            MockDeployGateway::new(deploy_result("test"), upsert_result(false, "unused"));

        let args = AppDeployArgs {
            visibility: Some(Visibility::Private),
            ..deploy_args(None, Some("test"))
        };

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].2, Some(Visibility::Private));
    }

    #[tokio::test]
    async fn deploy_with_path_and_no_name_returns_error_even_with_cached_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Updated").expect("write");

        let canonical_dir = dir.path().canonicalize().expect("canon");
        save_state(
            &canonical_dir,
            &DeployState::new(
                "id-1".to_string(),
                "cached-app".to_string(),
                Some("https://cached-app.app.statespace.com".to_string()),
                None,
            ),
        )
        .expect("save state");

        let (mock, create_calls, upsert_calls, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "cached-app"));

        let args = deploy_args(Some(dir.path().to_path_buf()), None);

        let error = run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect_err("missing --name should error");

        assert!(error.to_string().contains("Application name is required"));

        assert!(create_calls.lock().expect("lock").is_empty());
        assert!(upsert_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_with_path_uploads_non_markdown_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("assets")).expect("create assets dir");
        std::fs::write(dir.path().join("README.md"), "# Updated").expect("write readme");
        std::fs::write(dir.path().join("assets/config.json"), "{\"enabled\":true}")
            .expect("write json");

        let (mock, create_calls, upsert_calls, _, _) =
            MockDeployGateway::new(deploy_result("bar"), upsert_result(false, "bar"));

        let args = deploy_args(Some(dir.path().to_path_buf()), Some("bar"));

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        assert!(create_calls.lock().expect("lock").is_empty());

        let recorded_upserts = upsert_calls.lock().expect("lock");
        assert_eq!(recorded_upserts.len(), 1);
        let uploaded_paths: Vec<&str> = recorded_upserts[0]
            .1
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert!(uploaded_paths.contains(&"README.md"));
        assert!(uploaded_paths.contains(&"assets/config.json"));
    }

    #[tokio::test]
    async fn deploy_with_explicit_name_uses_upsert_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Updated").expect("write");

        let (mock, create_calls, upsert_calls, _, _) =
            MockDeployGateway::new(deploy_result("bar"), upsert_result(false, "bar"));

        let args = deploy_args(Some(dir.path().to_path_buf()), Some("bar"));

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        assert!(create_calls.lock().expect("lock").is_empty());

        let recorded_upserts = upsert_calls.lock().expect("lock");
        assert_eq!(recorded_upserts.len(), 1);
        assert_eq!(recorded_upserts[0].0, "bar");
    }

    #[tokio::test]
    async fn deploy_with_file_path_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("somefile.txt");
        std::fs::write(&file_path, "not a dir").expect("write");

        let (mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));

        let args = deploy_args(Some(file_path), Some("test-app"));

        let error = run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect_err("expected not-a-directory error");
        assert!(error.to_string().contains("Not a directory"));
    }

    #[tokio::test]
    async fn deploy_missing_readme_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");

        let (mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));

        let args = deploy_args(Some(dir.path().to_path_buf()), Some("test-app"));

        let error = run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect_err("expected missing README error");
        assert!(error.to_string().contains("README.md not found"));
    }

    #[tokio::test]
    async fn deploy_readme_is_directory_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("README.md")).expect("create README.md dir");

        let (mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));

        let args = deploy_args(Some(dir.path().to_path_buf()), Some("test-app"));

        let error = run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect_err("expected README.md dir error");
        assert!(error.to_string().contains("README.md not found"));
    }

    #[tokio::test]
    async fn deploy_without_path_name_taken_returns_suggestion() {
        let (mut mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));
        mock.fail_create_with_name_taken = true;

        let args = deploy_args(None, Some("taken-name"));

        let error = run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect_err("expected taken-name error");
        let message = error.to_string();
        assert!(message.contains("already taken"));
        assert!(message.contains("statespace deploy --name"));
    }

    #[tokio::test]
    async fn deploy_syncs_secrets_from_config_file_and_flags() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Hello").expect("write readme");

        std::fs::write(
            dir.path().join("config.toml"),
            "[env]\nFROM_APP = \"app-value\"\nSHARED = \"app\"\n",
        )
        .expect("write app config");

        let env_file_path = dir.path().join("deploy.env");
        std::fs::write(&env_file_path, "FROM_FILE=file-value\nSHARED=file\n")
            .expect("write env file");

        let (mut mock, _, _, set_secret_calls, delete_secret_calls) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));
        mock.existing_secret_keys = vec!["STALE".to_string(), "FROM_APP".to_string()];

        let args = AppDeployArgs {
            env_vars: vec![
                "SHARED=flag".to_string(),
                "FROM_FLAG=flag-value".to_string(),
            ],
            env_file: Some(env_file_path),
            ..deploy_args(Some(dir.path().to_path_buf()), Some("app"))
        };

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        let set_calls = set_secret_calls.lock().expect("lock");
        let set_map: BTreeMap<&str, &str> = set_calls
            .iter()
            .map(|(_, key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(set_map.get("FROM_APP"), Some(&"app-value"));
        assert_eq!(set_map.get("FROM_FILE"), Some(&"file-value"));
        assert_eq!(set_map.get("FROM_FLAG"), Some(&"flag-value"));
        assert_eq!(set_map.get("SHARED"), Some(&"flag"));

        let deleted_calls = delete_secret_calls.lock().expect("lock");
        assert_eq!(deleted_calls.len(), 1);
        assert_eq!(deleted_calls[0].1, "STALE");
    }

    #[tokio::test]
    async fn deploy_with_unchanged_files_and_unchanged_env_skips() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Hello").expect("write readme");
        std::fs::write(dir.path().join("config.toml"), "[env]\nA=\"1\"\n").expect("write config");

        let (first_mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));
        let first_args = deploy_args(Some(dir.path().to_path_buf()), Some("app"));
        run_deploy(first_args, Path::new(TEST_CONFIG_PATH), first_mock)
            .await
            .expect("first deploy");

        let (second_mock, create_calls, upsert_calls, set_secret_calls, delete_secret_calls) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));

        let second_args = deploy_args(Some(dir.path().to_path_buf()), Some("app"));
        run_deploy(second_args, Path::new(TEST_CONFIG_PATH), second_mock)
            .await
            .expect("second deploy");

        assert!(create_calls.lock().expect("lock").is_empty());
        assert!(upsert_calls.lock().expect("lock").is_empty());
        assert!(set_secret_calls.lock().expect("lock").is_empty());
        assert!(delete_secret_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_with_unchanged_files_but_changed_env_redeploys() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Hello").expect("write readme");
        std::fs::write(dir.path().join("config.toml"), "[env]\nA=\"1\"\n").expect("write config");

        let (first_mock, _, _, _, _) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));
        let first_args = deploy_args(Some(dir.path().to_path_buf()), Some("app"));
        run_deploy(first_args, Path::new(TEST_CONFIG_PATH), first_mock)
            .await
            .expect("first deploy");

        std::fs::write(dir.path().join("config.toml"), "[env]\nA=\"2\"\n").expect("rewrite config");

        let (second_mock, _, upsert_calls, set_secret_calls, _) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));

        let second_args = deploy_args(Some(dir.path().to_path_buf()), Some("app"));
        run_deploy(second_args, Path::new(TEST_CONFIG_PATH), second_mock)
            .await
            .expect("second deploy");

        assert_eq!(upsert_calls.lock().expect("lock").len(), 1);
        let set_calls = set_secret_calls.lock().expect("lock");
        assert!(
            set_calls
                .iter()
                .any(|(_, key, value)| key == "A" && value == "2")
        );
    }

    #[tokio::test]
    async fn deploy_does_not_auto_load_dotenv() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Hello").expect("write readme");
        std::fs::write(dir.path().join(".env"), "FROM_DOTENV=secret\n").expect("write dotenv");

        let (mock, _, _, set_secret_calls, _) =
            MockDeployGateway::new(deploy_result("app"), upsert_result(false, "app"));

        let args = deploy_args(Some(dir.path().to_path_buf()), Some("app"));

        run_deploy(args, Path::new(TEST_CONFIG_PATH), mock)
            .await
            .expect("run_deploy");

        assert!(set_secret_calls.lock().expect("lock").is_empty());
    }
}
