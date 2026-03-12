use crate::args::AppDeployArgs;
use crate::error::{ApiErrorCode, Error, GatewayError, Result};
use crate::gateway::GatewayClient;
use crate::gateway::applications::{ApplicationFile, DeployResult, UpsertResult, Visibility};
use crate::names::generate_name;
use crate::state::{DeployState, load_state, save_state};
use statespace_server::initialize_templates;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeployMode {
    Create,
    Upsert,
}

#[derive(Debug, Clone)]
struct DeployTarget {
    name: String,
    mode: DeployMode,
}

#[derive(Debug, Clone)]
struct DeployOutcome {
    created: bool,
    id: String,
    name: String,
    url: Option<String>,
    auth_token: Option<String>,
}

impl DeployOutcome {
    fn from_create(name: String, result: DeployResult) -> Self {
        Self {
            created: true,
            id: result.id,
            name,
            url: result.url,
            auth_token: result.auth_token,
        }
    }

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

pub(crate) async fn run_deploy(args: AppDeployArgs, gateway: impl DeployGateway) -> Result<()> {
    let visibility = args.visibility;

    match args.path {
        None => {
            // Create empty app (no files, no state file)
            let name = args.name.unwrap_or_else(generate_name);
            eprintln!("Creating empty application '{name}'...");

            let result = gateway
                .create_application(&name, Vec::new(), visibility)
                .await
                .map_err(|e| map_create_error(e, &name))?;

            eprintln!();
            eprintln!("Created '{name}'");
            eprintln!("  ID: {}", result.id);
            if let Some(ref url) = result.url {
                eprintln!("  URL: {url}");
            }
            if let Some(ref token) = result.auth_token {
                eprintln!("  Token: {token}");
            }

            Ok(())
        }
        Some(path) => {
            // Deploy files from directory
            let dir = path.canonicalize().map_err(|e| {
                crate::error::Error::cli(format!("Invalid path '{}': {e}", path.display()))
            })?;

            initialize_templates(&dir)
                .await
                .map_err(|e| Error::cli(format!("Failed to initialize templates: {e}")))?;

            if !dir.join("README.md").exists() {
                return Err(Error::cli(
                    "README.md not found. Create it before deploying your app.".to_string(),
                ));
            }

            let cached = load_state(&dir)?;
            let target = resolve_target(args.name, cached.as_ref());

            let files = GatewayClient::scan_deploy_files(&dir)?;

            if files.is_empty() {
                eprintln!("No files found in {}", dir.display());
                return Ok(());
            }

            let checksums: Vec<(String, String)> = files
                .iter()
                .map(|f| (f.path.clone(), f.checksum.clone()))
                .collect();

            if let Some(ref prev) = cached {
                let same_target = prev.name == target.name;
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

            let result = match target.mode {
                DeployMode::Create => {
                    let create_result = gateway
                        .create_application(&target.name, files, visibility)
                        .await
                        .map_err(|error| map_create_error(error, &target.name))?;
                    DeployOutcome::from_create(target.name.clone(), create_result)
                }
                DeployMode::Upsert => {
                    let upsert_result = gateway.upsert_application(&target.name, files).await?;
                    DeployOutcome::from_upsert(upsert_result)
                }
            };

            let action = if result.created { "Created" } else { "Updated" };
            eprintln!("{action} application '{}'", result.name);

            if let Some(ref url) = result.url {
                eprintln!("URL: {url}");
            }

            let state = DeployState::new(result.id, result.name, result.url, result.auth_token)
                .with_checksums(&checksums);

            save_state(&dir, &state)?;

            Ok(())
        }
    }
}

fn resolve_target(explicit_name: Option<String>, cached: Option<&DeployState>) -> DeployTarget {
    match explicit_name {
        Some(name) => {
            let mode = if cached.is_some_and(|state| state.name == name) {
                DeployMode::Upsert
            } else {
                DeployMode::Create
            };
            DeployTarget { name, mode }
        }
        None => match cached {
            Some(state) => DeployTarget {
                name: state.name.clone(),
                mode: DeployMode::Upsert,
            },
            None => DeployTarget {
                name: generate_name(),
                mode: DeployMode::Create,
            },
        },
    }
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

    type CreateCall = (String, Vec<ApplicationFile>, Option<Visibility>);
    type UpsertCall = (String, Vec<ApplicationFile>);
    type RecordedCreateCalls = Arc<Mutex<Vec<CreateCall>>>;
    type RecordedUpsertCalls = Arc<Mutex<Vec<UpsertCall>>>;

    struct MockDeployGateway {
        create_result: DeployResult,
        upsert_result: UpsertResult,
        fail_create_with_name_taken: bool,
        create_calls: RecordedCreateCalls,
        upsert_calls: RecordedUpsertCalls,
    }

    impl MockDeployGateway {
        fn new(
            create_result: DeployResult,
            upsert_result: UpsertResult,
        ) -> (Self, RecordedCreateCalls, RecordedUpsertCalls) {
            let create_calls = Arc::new(Mutex::new(Vec::new()));
            let upsert_calls = Arc::new(Mutex::new(Vec::new()));
            let mock = Self {
                create_result,
                upsert_result,
                fail_create_with_name_taken: false,
                create_calls: Arc::clone(&create_calls),
                upsert_calls: Arc::clone(&upsert_calls),
            };
            (mock, create_calls, upsert_calls)
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
                    message: format!("Environment name '{name}' is already taken"),
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

    #[tokio::test]
    async fn deploy_without_path_creates_empty_app() {
        let (mock, create_calls, upsert_calls) =
            MockDeployGateway::new(deploy_result("test-app"), upsert_result(false, "unused"));

        let args = AppDeployArgs {
            path: None,
            name: Some("test-app".to_string()),
            visibility: None,
        };

        run_deploy(args, mock).await.expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "test-app");
        assert!(recorded[0].1.is_empty()); // No files
        assert!(upsert_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_without_path_uses_random_name() {
        let (mock, create_calls, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));

        let args = AppDeployArgs {
            path: None,
            name: None,
            visibility: None,
        };

        run_deploy(args, mock).await.expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_ne!(recorded[0].0, "unused");
        assert!(recorded[0].0.contains('-'));
    }

    #[tokio::test]
    async fn deploy_without_path_passes_visibility() {
        let (mock, create_calls, _) =
            MockDeployGateway::new(deploy_result("test"), upsert_result(false, "unused"));

        let args = AppDeployArgs {
            path: None,
            name: Some("test".to_string()),
            visibility: Some(Visibility::Private),
        };

        run_deploy(args, mock).await.expect("run_deploy");

        let recorded = create_calls.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].2, Some(Visibility::Private));
    }

    #[tokio::test]
    async fn deploy_with_cached_state_uses_upsert() {
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

        let (mock, create_calls, upsert_calls) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "cached-app"));

        let args = AppDeployArgs {
            path: Some(dir.path().to_path_buf()),
            name: None,
            visibility: None,
        };

        run_deploy(args, mock).await.expect("run_deploy");

        assert!(create_calls.lock().expect("lock").is_empty());

        let recorded_upserts = upsert_calls.lock().expect("lock");
        assert_eq!(recorded_upserts.len(), 1);
        assert_eq!(recorded_upserts[0].0, "cached-app");

        let state = load_state(&canonical_dir)
            .expect("load")
            .expect("state exists");
        assert_eq!(state.name, "cached-app");
        assert_eq!(state.deployment_id, "id-1");
    }

    #[tokio::test]
    async fn deploy_with_path_uploads_non_markdown_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("assets")).expect("create assets dir");
        std::fs::write(dir.path().join("README.md"), "# Updated").expect("write readme");
        std::fs::write(dir.path().join("assets/config.json"), "{\"enabled\":true}")
            .expect("write json");

        let (mock, create_calls, upsert_calls) =
            MockDeployGateway::new(deploy_result("bar"), upsert_result(false, "bar"));

        let args = AppDeployArgs {
            path: Some(dir.path().to_path_buf()),
            name: Some("bar".to_string()),
            visibility: None,
        };

        run_deploy(args, mock).await.expect("run_deploy");

        let recorded_creates = create_calls.lock().expect("lock");
        assert_eq!(recorded_creates.len(), 1);
        let uploaded_paths: Vec<&str> = recorded_creates[0]
            .1
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert!(uploaded_paths.contains(&"README.md"));
        assert!(uploaded_paths.contains(&"assets/config.json"));
        assert!(upsert_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_with_explicit_name_creates_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Updated").expect("write");

        let (mock, create_calls, upsert_calls) =
            MockDeployGateway::new(deploy_result("bar"), upsert_result(false, "bar"));

        let args = AppDeployArgs {
            path: Some(dir.path().to_path_buf()),
            name: Some("bar".to_string()),
            visibility: None,
        };

        run_deploy(args, mock).await.expect("run_deploy");

        let recorded_creates = create_calls.lock().expect("lock");
        assert_eq!(recorded_creates.len(), 1);
        assert_eq!(recorded_creates[0].0, "bar");
        assert!(upsert_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn deploy_name_taken_returns_suggestion() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "# Hello").expect("write");

        let (mut mock, _, _) =
            MockDeployGateway::new(deploy_result("unused"), upsert_result(false, "unused"));
        mock.fail_create_with_name_taken = true;

        let args = AppDeployArgs {
            path: Some(dir.path().to_path_buf()),
            name: Some("taken-name".to_string()),
            visibility: None,
        };

        let error = run_deploy(args, mock)
            .await
            .expect_err("expected taken-name error");
        let message = error.to_string();
        assert!(message.contains("already taken"));
        assert!(message.contains("statespace deploy --name"));
    }
}
