# Kubernetes

Run the shop's arms on Kubernetes (GKE, EKS, AKS, or any other cluster). `ssp proxy
kubernetes` runs as its own Deployment in front of the shop. The shop's Deployment and
Service do not change.

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

When you add an arm, the proxy that holds the lease makes the control and the arm:

```shell
kubectl get deployments -l statespace.com/app=shop-prod
```

- `shop-ssp-control` runs a copy of live's image. `shop-ssp-<commit 12>` runs the
  image of the arm's build.
- Each copy has the shop's pod spec: its environment, Secrets, service account,
  resources, probes, and volumes. Only the image changes.
- The copies' pods carry only the labels `statespace.com/app` and
  `statespace.com/unit`, so the Service `shop` never sends them traffic.
- Each copy gets the shop's replicas times its share, at least one, and a Service of
  its own.
- The proxy reads live's image digest from the status of the shop's ready pods.

When an arm ends, the proxy deletes its Deployment and its Service. When live changes,
the proxy changes the control to the new image.

Kubernetes joins a container's standard output and standard error, so the proxy records
every line of a copy as `stdout`.
