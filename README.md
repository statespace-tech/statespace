<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/header-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/header-light.png">
    <img src="assets/header-light.png" alt="Statespace" width="520">
  </picture>
</div>

<div align="center">

<br>

[![CI](https://github.com/statespace-tech/statespace/actions/workflows/ci.yml/badge.svg)](https://github.com/statespace-tech/statespace/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/statespace?style=flat-square)](https://crates.io/crates/statespace)
[![License](https://img.shields.io/badge/license-Apache--2.0-007ec6?style=flat-square)](https://github.com/statespace-tech/statespace/blob/main/LICENSE)
[![Discord](https://img.shields.io/discord/1541944682727084143?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/qKEFpqG9mr)

</div>

Statespace runs commits of your app on small shares of live traffic, as the
arms of an experiment, and records what happens.

**Website:** [statespace.com](https://statespace.com)<br>
**Documentation:** [docs.statespace.com](https://docs.statespace.com)

## Install

```shell
curl -fsSL https://statespace.com/install.sh | bash
```

The command is `ssp`. `statespace` is the same program.

To let your coding agent do the setup, give it this prompt:

```text
Read https://statespace.com/skill.md and follow it.
```

## How it works

```text
                    ┌─► live service        (85%)  never recorded
visitors ─► proxy ──┼─► checkout:c0ffee1    ( 5%)  recorded
                    ├─► checkout:a1b2c3d    ( 5%)  recorded
                    └─► us-landing:b2c3d4e  ( 5%)  recorded
```

1. Your CI records the image of each commit with
   `statespace-tech/action@v1`.
2. You define experiments in `statespace.toml`, and give each one arms:
   commits with a share of traffic.
3. The proxy runs each arm beside live and sends it its share of visitors.
4. Statespace posts the evidence on each arm's commit as the status
   `statespace/<app>/<experiment>`. People and agents decide.

## Quickstart

1. Install the Statespace GitHub App on
   [statespace.com](https://statespace.com).
2. Write `statespace.toml` in the app's folder. Start from
   [the skill](skills/statespace/SKILL.md).
3. Add the action after your image build:

   ```yaml
   - uses: statespace-tech/action@v1
     with:
       image: ghcr.io/acme/shop@${{ steps.build.outputs.digest }}
   ```

4. Add the app and a runtime token, and run the proxy with the token:

   ```shell
   ssp app add shop-prod --note "shop, all traffic"
   ssp token add --app shop-prod --note "proxy"
   ```

5. Set the arms and start the experiment:

   ```shell
   ssp experiment update checkout \
     --arm c0ffee1=0.05 --arm a1b2c3d=0.05
   ssp experiment start checkout --note "one-page checkout"
   ```

The [quickstart](https://docs.statespace.com/quickstart) has every step in
full.

## Runtimes

```shell
ssp proxy docker      --app shop-dev  --image-file /etc/shop/image
ssp proxy cloudrun    --app shop-prod --service shop --region us-east1
ssp proxy kubernetes  --app shop-prod --deployment shop
ssp proxy fargate     --app shop-prod --cluster shop --service shop
ssp proxy fly         --app shop-prod --fly-app shop
```

Each runtime has a guide with its permissions, and an example in
[`examples/`](examples):
[Docker](https://docs.statespace.com/guides/docker),
[Cloud Run](https://docs.statespace.com/guides/cloud-run),
[Kubernetes](https://docs.statespace.com/guides/kubernetes),
[ECS Fargate](https://docs.statespace.com/guides/fargate),
[Fly.io](https://docs.statespace.com/guides/fly).

## Commands

```text
ssp app         add | remove | list | show
ssp experiment  list | show | update | start | stop
ssp token       add | remove | list
ssp proxy       docker | cloudrun | kubernetes | fargate | fly
ssp login | logout | whoami
```

See the [CLI reference](https://docs.statespace.com/reference/cli) and the
[`statespace.toml` reference](https://docs.statespace.com/reference/statespace-toml).

## License

Apache-2.0
