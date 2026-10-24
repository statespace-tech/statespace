# FastAPI shop

A small FastAPI shop that runs behind `ssp proxy`. It prints a JSON line for each
signup and purchase, with the subject and the amount. An arm is an `exp/` branch
of `main.py`, such as one that lowers `PRICE`, built into an image.

| File | Purpose |
| --- | --- |
| `main.py` | The app. It reads the subject from the `x-statespace-subject` header |
| `Dockerfile` | The shop's image: the default from `main`, each arm from an `exp/` branch |
| `statespace.toml` | The shop's app and the share of each arm |
| `statespace.yml` | A GitHub Actions workflow that publishes each commit of an `exp/` branch |
| `production.yml` | A GitHub Actions workflow that builds, records, and deploys the default |
| `ssp-proxy.service` | `ssp proxy` under systemd, on a machine with Docker |

For Kubernetes, see `../kubernetes`.

## Set up

1. Copy `statespace.toml` to the root of the shop's repository, and the two
   workflows to `.github/workflows/`.
2. On [statespace.com](https://statespace.com), select **Connect GitHub**, and
   install the Statespace GitHub App on the repository.
3. In branch protection for `main`, require one approval. Then a second person
   approves each commit before users see it.

## Run an experiment

Push a branch named `exp/price/lower-price` that lowers `PRICE`, and open a pull
request. CI publishes each commit as a version of the arm. When GitHub says that
the pull request can merge, 5% of the shop's subjects get the lower price and 5%
get the control. The `Statespace` check on the pull request tells what runs.

To stop the arm, make the pull request a draft. In an emergency, run
`ssp stop exp/<name>`, and `ssp resume exp/<name>` later. To ship it, merge it.

Compare revenue per subject in SQL, with a token from `ssp token create --name
sql --preset analyst --app shop`:

```sql
SELECT arm, count(DISTINCT subject_id) AS subjects,
       sum((data->>'amount')::numeric) / count(DISTINCT subject_id) AS revenue_per_subject
FROM shop.logs
WHERE experiment = 'price' AND reason = 'assigned'
GROUP BY arm;
```

`shop.logs` has one row per subject's run with each line the subject's arm
printed after it, so subjects without a purchase count with no revenue.

## Deploy

Install `ssp-proxy.service` wherever the shop runs now, with Docker. Give it a
runtime token from `ssp token create --name production --preset runtime --app
shop` as `SSP_TOKEN`. The proxy takes the app from the token. The proxy runs the image in `/etc/shop/image` as the default, and
each arm and control beside it. The containers inherit the service's
environment, such as `PAYMENT_KEY` from `/etc/shop/env`.

To deploy a new default, write its image to `/etc/shop/image` and run `systemctl
reload shop`. The proxy starts the new image, sends it traffic once `/health`
answers, and then stops the old container.

Behind a load balancer, run as many machines as before. Every proxy assigns a
subject the same arm, so requests need no stickiness. Each proxy runs the
default and up to `--max-arms` arms, 5 by default, controls included. Size
machines for that many copies of the shop, or lower the limit.
