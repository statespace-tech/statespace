# Auth Service

**Owner:** Identity team (`#team-identity` on Slack)
**Repo:** `github.com/acme/auth-service`
**Language:** Go

## Overview

Handles authentication and authorization for all Acme products. Issues JWTs with a 15-minute access token TTL and 7-day refresh token TTL.

## Config

```yaml
auth:
  issuer: https://auth.acme.com
  token_ttl: 900          # seconds
  refresh_ttl: 604800     # seconds
  bcrypt_cost: 12
  max_login_attempts: 5
  lockout_duration: 1800  # seconds

oauth:
  providers:
    - google
    - github
    - okta
  callback_url: https://auth.acme.com/oauth/callback

rate_limits:
  login: 10/min
  token_refresh: 30/min
  password_reset: 3/hour
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | /auth/login | Email + password login |
| POST | /auth/refresh | Refresh access token |
| POST | /auth/logout | Revoke refresh token |
| GET | /oauth/{provider} | Start OAuth flow |
| GET | /oauth/callback | OAuth callback |
| POST | /auth/reset-password | Request password reset |

## Dependencies

- PostgreSQL (dedicated `auth_db`)
- Redis (session cache, rate limiting)
- AWS SES (password reset emails)
