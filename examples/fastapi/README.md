# FastAPI shop

A small FastAPI shop that runs behind `ssp proxy`. It prints a JSON line for each signup
and purchase. An arm is a commit of `main.py`, such as one that lowers `PRICE`, built
into an image.

| File | Purpose |
| --- | --- |
| `main.py` | The app |
| `Dockerfile` | The shop's image. CI builds it for every commit |
| `statespace.toml` | The settings, the events, the metrics, and the experiment `price` |
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
2. Get the live commit, and put it and the new commit in the experiment `price`:

   ```shell
   ssp app show shop-prod
   ssp experiment update price --app shop-prod \
     --arm <live commit>=0.05 --arm <new commit>=0.05
   ```

   The live commit is the reference. 5% of the shop's subjects get it, and 5% get
   the lower price. Everyone else gets live.
3. Start the experiment. The command waits until both arms are ready.

   ```shell
   ssp experiment start price --app shop-prod --note "lower price"
   ```

4. Read the evidence: the commit status `statespace/shop-prod/price`, or

   ```shell
   ssp experiment show price --app shop-prod
   ```

5. Stop the experiment when you decide, or at once in an emergency:

   ```shell
   ssp experiment stop price --app shop-prod --note "enough data"
   ```

## Deploy

To deploy a new live, write its image to `/etc/shop/image`. The proxy starts the new
image, sends it traffic once `/health` answers, and stops the old container. The arms
do not change: they are fixed commits.

Each machine runs live and one container for each arm of each running experiment.
Size machines for that many copies of the shop, or keep few arms.
