# Payments Service

**Owner:** Revenue team (`#team-revenue` on Slack)
**Repo:** `github.com/acme/payments-service`
**Language:** Python (FastAPI)

## Overview

Manages subscriptions, one-time charges, and invoicing. Integrates with Stripe as the payment processor.

## Config

```yaml
payments:
  currency: usd
  stripe_api_version: "2024-04-10"
  webhook_tolerance: 300  # seconds
  retry_attempts: 3
  retry_backoff: exponential

subscriptions:
  plans:
    - id: free
      price: 0
      limits: { api_calls: 1000, storage_gb: 1 }
    - id: pro
      price: 49
      limits: { api_calls: 50000, storage_gb: 50 }
    - id: enterprise
      price: 199
      limits: { api_calls: unlimited, storage_gb: 500 }
  trial_days: 14
  grace_period_days: 7

invoicing:
  net_terms: 30
  auto_charge: true
  tax_provider: avalara
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /subscriptions | Create subscription |
| GET | /subscriptions/{id} | Get subscription details |
| PATCH | /subscriptions/{id} | Update plan |
| DELETE | /subscriptions/{id} | Cancel subscription |
| GET | /invoices | List invoices |
| POST | /webhooks/stripe | Stripe webhook receiver |

## Dependencies

- PostgreSQL (dedicated `payments_db`)
- Stripe API
- Avalara (tax calculation)
- Kafka (emits `payment.completed`, `subscription.changed` events)
