# Kubernetes

Run the FastAPI shop's experiments on Kubernetes (GKE, EKS, AKS, or any
other cluster). Each pod of the shop runs `ssp proxy` beside the shop, and one
`ssp controller` pod runs every arm as a copy of the shop's Deployment.

| File | Purpose |
| --- | --- |
| `shop.yaml` | The shop's Deployment with the `statespace-proxy` sidecar, and its Service |
| `statespace-controller.yaml` | The controller, its service account, and what it may do |

## Set up

Create a runtime token for the app, and store it in a Secret. The proxies and
the controller use the same token, and take the app from it.

```shell
ssp token create --name shop --preset runtime --app shop
kubectl create secret generic statespace \
  --from-literal=token=<token>
kubectl apply -f shop.yaml -f statespace-controller.yaml
```

CI publishes each commit of an `exp/` branch, as `../fastapi/statespace.yml`
does. When the branch's pull request can merge, the controller starts its arm
and the arm's control:

```shell
kubectl get deployments -l app.kubernetes.io/managed-by=ssp-controller
```

## What the controller makes

For each arm of a running experiment, the controller creates a Deployment and
a Service named after the shop, the experiment, and the arm:

- The pods copy the shop's pod spec: its environment, Secrets, service
  account, resources, probes, and volumes. Only the image differs, and the
  `statespace-proxy` sidecar is left out.
- The pods carry only the controller's labels, so the shop's Service never
  sends them traffic. The proxies send them their subjects.
- Each arm gets the shop's replicas times its share of the subjects, at least
  one.
- A control runs the default build of its arms' base commit, which the shop ran
  as its default.

When an arm stops, its Deployment and Service are deleted. Deleting the controller's
Deployment deletes every arm it made.

Arms record every line their pods print. Kubernetes merges a container's
standard output and standard error, so every line is recorded as `stdout`.
