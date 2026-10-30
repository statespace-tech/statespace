# Fly.io

Run the arms of the shop's experiments on Fly.io. `ssp proxy fly` runs as
its own app, `shop-statespace`, in front of the app `shop`, in the same
organization. Each arm is a Machine of `shop` without services.

## Set up

1. Make `shop` listen on `[::]`, so that the proxy reaches its Machines on
   the private network. Deploy it.
2. Push the builds of `shop` to `registry.fly.io/shop`, so that Fly.io can
   pull the image of each arm.
3. Give `shop` a private Flycast address. The proxy sends live traffic to
   `http://shop.flycast`:

   ```shell
   fly ips allocate-v6 --private -a shop
   ```

4. Make the proxy's app:

   ```shell
   fly apps create shop-statespace
   ```

5. Give the proxy a runtime token and a deploy token of `shop`. A deploy
   token can manage the Machines of `shop` only. Renew it before it
   expires:

   ```shell
   fly secrets set -a shop-statespace --stage \
     STATESPACE_TOKEN="$(ssp token add --app shop-prod \
       --note "Fly.io proxy" --json | jq -r .token)" \
     FLY_API_TOKEN="$(fly tokens create deploy -a shop -x 2160h)"
   ```

6. Deploy the proxy with [`fly.toml`](fly.toml):

   ```shell
   fly deploy -c examples/fly/fly.toml
   ```

7. Point the domain of the shop at `shop-statespace`. Then remove the public
   addresses of `shop`, so that all visitors go through the proxy:

   ```shell
   fly ips list -a shop
   fly ips release -a shop <public address>
   ```

Never set a secret that starts with `STATESPACE_` on `shop`: the arms get
the secrets of `shop`.

## What the proxy does

- Live is the Machines of `shop` in the process group `app` that the proxy
  did not make. Live's image is the digest that they all run.
- For each arm of a starting or running experiment, the proxy copies the
  config of a live Machine, changes the image, and makes a Machine named
  `ssp-<experiment>-<commit 12>`, such as `ssp-checkout-a1b2c3d4e5f6`.
- The copy has no services, so Fly.io never sends it public or Flycast
  traffic. It has no mounts and no `fly_*` metadata, so `fly deploy` does
  not change it.
- The proxy sends each request of an arm to the Machine's private IPv6
  address.
- The proxy reads the output of the arms from the logs of `shop`.
- When an experiment stops, the proxy destroys the Machines of its arms.

`fly deploy` of `shop` changes live only. The arms are fixed commits. A new
live does not change them.
