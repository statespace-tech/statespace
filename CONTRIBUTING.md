# Contributing

Thank you for helping build Statespace.

Open an issue before a substantial feature or a change to a public command, so we can agree
on the interface first. Small fixes can go straight to a pull request.

Do not report security issues publicly. Follow [SECURITY.md](SECURITY.md).

## Development

Install stable Rust and run the checks that CI runs.

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
```

Pushes run only the fast tests. The runtime tests run `ssp proxy` on each runtime for
real. They run before each release (`.github/workflows/runtime-tests.yml`). An admin
can also run them on `main` with "Run workflow". You can run them on your machine too.

The Docker test runs `ssp proxy docker` with real containers, a local registry, and a
stand-in for Statespace. It needs Docker.

```shell
STATESPACE_DOCKER_TESTS=1 cargo test --locked --test cli docker
```

The Kubernetes test runs `ssp proxy kubernetes` against a kind cluster. Select the
cluster in kubectl first.

```shell
kind create cluster --name ssp-test
STATESPACE_KIND_TESTS=1 cargo test --locked --test cli kind
```

The Cloud Run tests use stand-ins for the Admin API, the metadata server, and Cloud
Logging. They run with `cargo test`.

The real Cloud Run test runs `ssp proxy cloudrun` in a sandbox project of Google
Cloud. Use only a sandbox project. Never use a production project or a project of a
customer. The test does these steps:

1. It makes sure that the Application Default Credentials are of
   `STATESPACE_TEST_GCP_PROJECT`. If not, it stops before it makes anything.
2. It pushes three images of `examples/app` to the Artifact Registry repository
   `statespace-test`. It deploys one image as the service `statespace-test-<run id>`.
3. It runs the proxy on your machine with your credentials. It starts an experiment
   with two arms, deploys a new live, and stops the experiment. Then it does the same
   with the proxy as its own Cloud Run service.
4. It deletes its services and images, also when it fails. The sweeper deletes the
   services and images of runs that are older than three hours.

The test needs Docker and a service account of the sandbox that runs the services.
Your credentials must have the role Service Account Token Creator on that account. To
run the proxy as a Cloud Run service, the test also needs a Linux amd64 build of `ssp`
and `cloudflared`.

```shell
gcloud auth application-default login
export STATESPACE_CLOUDRUN_TESTS=1
export STATESPACE_TEST_GCP_PROJECT=<sandbox project>
export STATESPACE_TEST_GCP_REGION=us-east1
export STATESPACE_TEST_GCP_SERVICE_ACCOUNT=<account>@<sandbox project>.iam.gserviceaccount.com
# Optional: run the proxy as a Cloud Run service too.
cargo build --release --locked --target x86_64-unknown-linux-musl
export STATESPACE_TEST_SSP_BINARY=$PWD/target/x86_64-unknown-linux-musl/release/ssp
cargo test --locked --test cli gcp_sweeper -- --nocapture
cargo test --locked --test cli gcp_cloudrun -- --nocapture
cargo test --locked --test cli gcp_sweeper -- --nocapture
```

The Fargate tests use stand-ins for ECS, EC2, and CloudWatch Logs. They run
with `cargo test`.

The real Fargate test runs `ssp proxy fargate` in a sandbox account of AWS.
Use only a sandbox account. Never use a production account or an account of
a customer. The test does these steps:

1. It makes sure that the AWS credentials are of
   `STATESPACE_TEST_AWS_ACCOUNT`. If not, it stops before it makes anything.
2. It uses the default VPC. When the account has no default VPC, it makes a
   small VPC with public subnets and an internet gateway, without a NAT
   gateway.
3. It makes a security group that lets only your IP address connect to
   port 8000, an ECR repository, a log group, and a cluster. It pushes three
   images of `examples/app`, and runs one as the service. Each name is
   `statespace-test-<run id>`.
4. It runs the proxy on your machine with your credentials, and with
   `--unit-ip public`. It starts an experiment with two arms, deploys a new
   live, and stops the experiment.
5. It deletes what it made, also when it fails. The sweeper deletes what
   runs older than three hours left.

The test needs Docker and the AWS CLI. The tasks use the execution role
`statespace-test-task-execution` of the account.

```shell
export STATESPACE_FARGATE_TESTS=1
export STATESPACE_TEST_AWS_ACCOUNT=<sandbox account>
export STATESPACE_TEST_AWS_REGION=us-east-1
cargo test --locked --test cli aws_sweeper -- --nocapture
cargo test --locked --test cli aws_fargate -- --nocapture
cargo test --locked --test cli aws_sweeper -- --nocapture
```

Run the CLI against a local service by setting its URL.

```shell
STATESPACE_API=http://localhost:8080 cargo run -- app list
```

## Pull requests

- Keep each pull request focused on one change and explain the user-visible result.
- Add or update tests and the README for public behavior.
- Add an entry to `CHANGELOG.md` for a user-visible change.
- Use [Conventional Commits](https://www.conventionalcommits.org) for commit messages.

## Releases

Set the version in `Cargo.toml`. Update `CHANGELOG.md`. Then push a `v` tag. The release
workflow builds an archive for each platform. It publishes the GitHub release and the
`ghcr.io/statespace-tech/statespace` image, and publishes the crate. Then statespace.com
serves the installer and the agent skill of the release.

```shell
git tag v0.4.0 && git push origin v0.4.0
```

By contributing, you agree that your contribution is licensed under the Apache License 2.0.
