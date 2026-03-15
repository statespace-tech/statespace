# Kubernetes

## Clusters

| Cluster | Region | Version | Nodes |
|---------|--------|---------|-------|
| prod-us | us-east-1 | 1.29 | 12 |
| prod-eu | eu-west-1 | 1.29 | 8 |
| staging | us-east-1 | 1.30 | 4 |

## Namespaces

- `default` — never used
- `core` — auth, payments, gateway
- `data` — analytics pipeline, ETL jobs
- `monitoring` — Prometheus, Grafana, alertmanager

## Resource Defaults

All deployments must specify requests and limits:

```yaml
resources:
  requests:
    cpu: 100m
    memory: 128Mi
  limits:
    cpu: 500m
    memory: 512Mi
```

## Scaling Policy

- HPA enabled on all `core` namespace services (min 2, max 20 replicas)
- Cluster autoscaler active on `prod-us` and `prod-eu` (max 30 nodes)
- Staging does not autoscale — manually adjust via `kubectl scale`
