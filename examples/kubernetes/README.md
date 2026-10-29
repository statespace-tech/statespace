# Kubernetes

Run the arms of the shop's experiments on Kubernetes (GKE, EKS, AKS, or any other
cluster). `ssp proxy kubernetes` runs as its own Deployment in front of the shop. The
shop's Deployment and Service do not change.

| File | Purpose |
| --- | --- |
| `shop.yaml` | The shop's Deployment and Service |
| `statespace-proxy.yaml` | The proxy's Deployment and Service, its service account, and its Role |

## Set up

1. Add a runtime token for the app, and store it in a Secret:

   ```shell
   ssp token add --app shop-prod --note "Kubernetes proxy"
   kubectl create secret generic statespace --from-literal=token=<token>
   kubectl apply -f shop.yaml -f statespace-proxy.yaml
   ```

2. Point the Ingress or the load balancer at the Service `statespace-proxy`.
3. Check that the proxy reports, and that the app has a live commit:

   ```shell
   ssp app show shop-prod
   ```

## What the proxy makes

When you start an experiment, the proxy that holds the lease makes one copy of the
shop for each arm:

```shell
kubectl get deployments -l statespace.com/experiment=checkout
```

- The name of a copy is `shop-ssp-<experiment>-<commit 12>`, such as
  `shop-ssp-checkout-a1b2c3d4e5f6`. A name has at most 63 characters. A longer name
  is cut and ends with a short hash.
- Each copy runs the image of its arm's build.
- Each copy has the shop's pod spec: its environment, Secrets, service account,
  resources, probes, and volumes. Only the image changes.
- The copies' pods carry only the labels `statespace.com/app`,
  `statespace.com/unit`, `statespace.com/experiment`, and `statespace.com/commit`.
  Thus the Service `shop` never sends them traffic.
- Each copy gets the shop's replicas times its share, at least one, and a Service of
  its own.
- One commit in two experiments runs as two copies.
- The proxy reads live's image digest from the status of the shop's ready pods.

When an experiment stops, the proxy deletes the Deployments and Services of its
arms.

Kubernetes joins a container's standard output and standard error, so the proxy records
every line of a copy as `stdout`.
