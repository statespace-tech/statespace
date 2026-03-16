# Incident Response

## Severity Levels

| Level | Criteria | Response Time | Example |
|-------|----------|---------------|---------|
| SEV-1 | Full outage, data loss risk | 15 min | Production DB down |
| SEV-2 | Degraded service for >10% users | 30 min | Auth latency >5s |
| SEV-3 | Minor impact, workaround exists | 4 hours | Dashboard rendering bug |
| SEV-4 | No user impact | Next business day | Flaky test in CI |

## On-Call Escalation

1. **Primary on-call** — PagerDuty alert fires, acknowledge within SLA
2. **Secondary on-call** — Auto-escalated after 10 min if not acknowledged
3. **Engineering manager** — Auto-escalated after 20 min for SEV-1/SEV-2
4. **VP Engineering** — Manually escalated for SEV-1 lasting >1 hour

## During an Incident

1. Join `#incident-room` on Slack
2. Claim the incident in PagerDuty
3. Post a status update every 15 min (SEV-1) or 30 min (SEV-2)
4. If customer-facing, notify `#support-escalations` for comms
5. When mitigated, update status page and post summary to `#incidents`

## Postmortem

Required for all SEV-1 and SEV-2 incidents. Due within 5 business days.

Template: `docs/templates/postmortem.md` in the monorepo.

Sections: timeline, root cause, impact, action items (each with owner and due date).

Blameless — focus on systems, not individuals.
