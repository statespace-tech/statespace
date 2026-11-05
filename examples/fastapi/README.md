# FastAPI shop

A small FastAPI shop that runs behind `ssp proxy`. It names each purchase in a response
header. An arm is a commit of `main.py`, such as one that lowers `PRICE`, built
into an image.

| File | Purpose |
| --- | --- |
| `main.py` | The app |
| `Dockerfile` | The shop's image. CI builds it for every commit |
| `statespace.toml` | The settings, the events, and the experiment `price` |
| `build.yml` | A GitHub Actions workflow that builds every commit and records the build |
| `compose.yaml` | The shop and `ssp proxy docker` with Docker Compose |

For Kubernetes, see `../kubernetes`. For Cloud Run, see `../cloudrun`.

## Set up

1. Install the Statespace GitHub App on the repository from
   [statespace.com](https://statespace.com).
2. Copy `statespace.toml` to the root of the shop's repository, and `build.yml` to
   `.github/workflows/`. Push.
3. Add the app, and a runtime token for the proxy:

   ```shell
   ssp app add shop-prod --note "shop, all traffic"
   ssp token add --app shop-prod --note "Compose proxy"
   ```

4. Put the token in `.env` as `STATESPACE_TOKEN=...`, next to `compose.yaml`, and
   start the project with `docker compose up -d`. You run the shop as live. The
   proxy forwards live traffic to the container `shop`, and runs each arm as a copy
   of that container with another image.

## Test a change

1. Commit a change that lowers `PRICE`, and push it. CI builds and records it.
2. Get the live commit, and put it and the new commit in the experiment `price`:

   ```shell
   ssp app show shop-prod
   ssp experiment update price --app shop-prod \
     --arm <live commit>=0.05 --arm <new commit>=0.05
   ```

   The live commit is an arm too. 5% of the shop's subjects get it, and 5% get
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

To deploy a new live, change the image of `shop` in `compose.yaml` and run
`docker compose up -d`. Compose creates the container `shop` again, and the proxy
sends live traffic to the new container within seconds. The arms do not change: they
are fixed commits.

The arms have no Compose labels, so `docker compose up --remove-orphans` does not
remove them. When the proxy stops, it removes them.

The machine runs live and one container for each arm of each running experiment.
Size the machine for that many copies of the shop, or keep few arms.
