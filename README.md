<br>

<div align="center">
  <a href="https://statespace.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/header-light.svg">
      <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/header-dark.svg">
      <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/header-dark.svg" alt="Statespace" width="520">
    </picture>
  </a>
</div>

<div align="center">

<br>

[![CI](https://img.shields.io/github/actions/workflow/status/statespace-tech/statespace/ci.yml?style=flat-square&label=CI)](https://github.com/statespace-tech/statespace/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/statespace?style=flat-square)](https://crates.io/crates/statespace)
[![License](https://img.shields.io/badge/license-Apache--2.0-007ec6?style=flat-square)](https://github.com/statespace-tech/statespace/blob/main/LICENSE)
[![Discord](https://img.shields.io/discord/1541944682727084143?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/qKEFpqG9mr)

</div>

Statespace lets you A/B test code changes on your own infrastructure. A reverse proxy runs beside your app and routes a small share of your traffic to experimental versions of it. Statespace tags their logs, traces, and requests before they land in your own data tools so your agents can compare and act on them.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/how-it-works-light.svg">
  <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/how-it-works-dark.svg">
  <img src="https://raw.githubusercontent.com/statespace-tech/statespace/main/assets/how-it-works-dark.svg" alt="Subjects go through the proxy: 80% to main, and 10% to each of the commits c0ffee1 and a1b2c3d. All of it goes on to your own data tools, such as Datadog, Grafana, Sentry, PostHog, Postgres, OpenTelemetry, Google Analytics, Supabase, and Elasticsearch." width="100%">
</picture>


<p align="center">
  <a href="https://docs.statespace.com/guides/docker">Docker</a> ·
  <a href="https://docs.statespace.com/guides/kubernetes">Kubernetes</a> ·
  <a href="https://docs.statespace.com/guides/cloud-run">Cloud Run</a> ·
  <a href="https://docs.statespace.com/guides/fargate">ECS Fargate</a> ·
  <a href="https://docs.statespace.com/guides/fly">Fly.io</a> ·
  <a href="https://docs.statespace.com/guides/s3">S3</a> ·
  <a href="https://docs.statespace.com/guides/gcs">Cloud Storage</a>
</p>

## Quickstart

1. Install the CLI and sign in with GitHub:

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ssp login
   ```

2. Install the
   [GitHub App](https://github.com/apps/statespace-tech/installations/new)
   and add this step to your image build:

   ```yaml
   permissions:
     contents: read
     packages: write
     id-token: write

   steps:
     # ... your steps that build and push the image (id: build) ...
     - uses: statespace-tech/action@v1
       with:
         image: ghcr.io/acme/shop@${{ steps.build.outputs.digest }}
   ```

3. Add `statespace.toml` to your repo and define your experiment:

   ```toml
   [[event]]
   name  = "signup"
   match = 'request.method == "POST" && request.path == "/signup"'

   [[event]]
   name  = "search"
   match = 'request.path == "/search"'

   [experiment.checkout]
   subject     = 'request.headers["x-user-id"]'
   eligibility = 'request.headers["cf-ipcountry"] == "US"'  # behind Cloudflare
   ```

4. Create your app and its proxy token:

   ```shell
   ssp app add shop-prod
   ssp token add --app shop-prod
   ```

5. Run the proxy next to your app and point your traffic at it:

   ```shell
   STATESPACE_TOKEN=<token> ssp proxy kubernetes --deployment shop
   ```

6. Give two commits a share of traffic and start:

   ```shell
   ssp experiment update checkout --arm c0ffee1=0.1 --arm a1b2c3d=0.1
   ssp experiment start checkout --note "one-page checkout"
   ```

7. Compare the versions:

   ```text
   $ ssp experiment show checkout
   checkout v1 · shop-prod · since 2026-11-04 09:12 UTC · 2,392 subjects · split ok

   latency (ms)                 c0ffee1                   a1b2c3d
     p50                        41                        44
     p95                        180                       205
     p99                        420                       610

   events (estimate [95% CI])   c0ffee1                   a1b2c3d
     requests_4xx               0.1121 [0.0950, 0.1318]   0.1178 [0.1003, 0.1378]
     requests_5xx               0.0274 [0.0193, 0.0388]   0.0438 [0.0331, 0.0578]
     signup                     0.0208 [0.0136, 0.0306]   0.0261 [0.0184, 0.0368]
     search                     0.2804 [0.2554, 0.3068]   0.2753 [0.2499, 0.3007]

   Find it in your own tools: statespace.arm = "c0ffee1…" | "a1b2c3d…"
   ```

## Community

Ask questions on [Discord](https://discord.gg/qKEFpqG9mr), and report bugs
in [issues](https://github.com/statespace-tech/statespace/issues). See
[CONTRIBUTING.md](https://github.com/statespace-tech/statespace/blob/main/CONTRIBUTING.md) for more info.

## License

Apache-2.0
