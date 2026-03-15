# Networking

## VPC Layout

| VPC | CIDR | Region | Purpose |
|-----|------|--------|---------|
| vpc-prod-us | 10.0.0.0/16 | us-east-1 | Production US |
| vpc-prod-eu | 10.1.0.0/16 | eu-west-1 | Production EU |
| vpc-staging | 10.2.0.0/16 | us-east-1 | Staging |

Subnets are split into public (ALB), private (services), and isolated (databases).

## DNS

- External: Route53 → `*.acme.com`
- Internal: CoreDNS in-cluster → `<service>.<namespace>.svc.cluster.local`
- Service mesh: Istio handles east-west traffic between namespaces

## Load Balancers

- **External ALB** — terminates TLS, routes to Istio ingress gateway
- **Internal NLB** — gRPC traffic between VPCs over PrivateLink
- Rate limits: 1000 req/s per IP on external ALB, no limit internal

## Egress

All outbound traffic goes through a NAT gateway with a static IP pool. Third-party allowlists use these IPs:

- us-east-1: `52.10.20.30/32`, `52.10.20.31/32`
- eu-west-1: `34.240.50.60/32`, `34.240.50.61/32`
