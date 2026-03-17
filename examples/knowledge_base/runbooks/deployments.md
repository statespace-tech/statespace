# Deployments

## Release Process

1. Merge PR to `main` — CI runs tests and builds container image
2. Image tagged `main-<sha>` and pushed to ECR
3. ArgoCD detects new image and syncs to **staging** automatically
4. QA validates in staging (automated smoke tests + manual spot check)
5. Engineer promotes to **prod-us** via ArgoCD UI or `argocd app sync prod-us`
6. Monitor dashboards for 15 min, then sync **prod-eu**

Deploys are blocked during the merge freeze window (Tuesdays and Thursdays 12:00–14:00 UTC).

## Rollback

**Fast rollback** (< 5 min):

1. Open ArgoCD → select app → History
2. Click previous healthy revision → Sync
3. Post in `#deploys` that you rolled back and why

**Database rollback** — if the release included a migration:

1. Check if migration is reversible (`down` migration exists)
2. Run `make db-rollback env=prod` from the service repo
3. Then rollback the application as above

## Feature Flags

We use LaunchDarkly for feature flags. Rules:

- All new user-facing features must be behind a flag
- Flags are cleaned up within 30 days of full rollout
- Flag naming: `team.feature-name` (e.g., `revenue.new-checkout-flow`)
- Emergency kill switches: prefixed `kill.` and always default to OFF
