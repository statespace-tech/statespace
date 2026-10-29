# FastAPI shop

A small FastAPI shop that runs behind `ssp proxy`. It prints a JSON line for each signup
and purchase. An arm is a commit of `main.py`, such as one that lowers `PRICE`, built
into an image.

| File | Purpose |
| --- | --- |
| `main.py` | The app |
| `Dockerfile` | The shop's image. CI builds it for every commit |
| `statespace.toml` | The settings, the events, and the metrics |
| `build.yml` | A GitHub Actions workflow that builds every commit and records the build |
| `ssp-proxy.service` | `ssp proxy docker` under systemd, on a machine with Docker |

For Kubernetes, see `../kubernetes`. For Cloud Run, see `../cloudrun`.

## Set up

1. Install the Statespace GitHub App on the repository from
   [statespace.com](https://statespace.com).
2. Copy `statespace.toml` to the root of the shop's repository, and `build.yml` to
   `.github/workflows/`. Push.
3. Add the app, and a runtime token for the proxy:

   ```shell
   ssp app add shop-prod --note "shop, all traffic"
   ssp token add --app shop-prod --note "systemd proxy"
   ```

4. Install `ssp-proxy.service` where the shop runs now, with the token in
   `/etc/shop/statespace`. The proxy runs the image in `/etc/shop/image` as live.

## Test a change

1. Commit a change that lowers `PRICE`, and push it. CI builds and records it.
2. Add the commit as an arm:

   ```shell
   ssp arm add <commit> --app shop-prod --share 0.05 --note "lower price"
   ```

   5% of the shop's subjects get the lower price, and 5% get the control.
3. Read the evidence: the commit status `statespace/shop-prod`, or

   ```shell
   ssp arm show <commit> --app shop-prod
   ```

4. In an emergency, stop it with `ssp arm stop <commit> --app shop-prod`. When you
   decide, end it with `ssp arm remove <commit> --app shop-prod`.

## Deploy

To deploy a new live, write its image to `/etc/shop/image`. The proxy starts the new
image, sends it traffic once `/health` answers, and stops the old container. The
control follows: the proxy replaces it with a copy of the new live image.

Each machine runs live, the control, and up to `max_arms` arms. Size machines for that
many copies of the shop, or lower `max_arms` in `statespace.toml`.
