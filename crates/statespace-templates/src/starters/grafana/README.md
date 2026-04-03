---
tools:
  - [curl, -sS, -G, -H, "Authorization: Bearer $GRAFANA_TOKEN", $LOKI_QUERY_RANGE_URL]
  - [curl, -sS, -G, -H, "Authorization: Bearer $GRAFANA_TOKEN", $TEMPO_SEARCH_URL]
---

# Grafana Incident Triage
- Use this template for questions like "we saw elevated 500s from 1pm to 3pm, can we check it out?"
- This app exposes two read-only tools: one for Loki logs and one for Tempo traces.
- Start with Loki to find the failing service, route, or error family. Pivot to Tempo only after the logs narrow the search.
- Prefer `since=...` for fast triage. If the user gives an exact window, replace `since=...` with explicit `start=...` and `end=...` values supported by your Grafana proxy.
- `grafana cli` is intentionally not used here because it is for Grafana server administration, not Loki or Tempo incident queries.
- Reference [Loki HTTP API docs](https://grafana.com/docs/loki/latest/reference/loki-http-api/) and [TraceQL docs](https://grafana.com/docs/tempo/latest/traceql/) as needed.

## Start Here
1. Run the app locally:

```bash
statespace run --env-file .env .
```

2. Read the root page:

```bash
curl http://127.0.0.1:8000/README.md
```

3. Execute tools by POSTing back to `/README.md`.

## Suggested Agent Prompt

```text
Use the app at http://127.0.0.1:8000/README.md.
We saw elevated 500s from 1pm to 3pm UTC for gateway.
Start with Loki logs, narrow to the failing route or error family, then pivot to Tempo traces if needed.
Summarize what changed and the likely root cause.
```

## Common Recipes

### Recent Gateway Logs

Use this when you want a quick feel for what the service is doing before you focus on errors.

```bash
curl -X POST http://127.0.0.1:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command":["curl","-sS","-G","-H","Authorization: Bearer $GRAFANA_TOKEN","$LOKI_QUERY_RANGE_URL","--data-urlencode","query={service_name=\"gateway\"}","--data-urlencode","since=15m","--data-urlencode","limit=20"]}'
```

### Elevated 500s

Start with the simple literal filter. It works even when logs are not structured JSON.

```bash
curl -X POST http://127.0.0.1:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command":["curl","-sS","-G","-H","Authorization: Bearer $GRAFANA_TOKEN","$LOKI_QUERY_RANGE_URL","--data-urlencode","query={service_name=\"gateway\"} |= \"500\"","--data-urlencode","since=2h","--data-urlencode","limit=50"]}'
```

If the logs are structured JSON, this usually gives a cleaner signal:

```bash
curl -X POST http://127.0.0.1:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command":["curl","-sS","-G","-H","Authorization: Bearer $GRAFANA_TOKEN","$LOKI_QUERY_RANGE_URL","--data-urlencode","query={service_name=\"gateway\"} | json | status_code >= 500","--data-urlencode","since=2h","--data-urlencode","limit=50"]}'
```

### Route Or Message Follow-Up

Once you know the suspicious route or string, add a literal filter.

```bash
curl -X POST http://127.0.0.1:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command":["curl","-sS","-G","-H","Authorization: Bearer $GRAFANA_TOKEN","$LOKI_QUERY_RANGE_URL","--data-urlencode","query={service_name=\"gateway\"} |= \"telemetry init failed\"","--data-urlencode","since=24h","--data-urlencode","limit=50"]}'
```

### Pivot To Traces

After Loki identifies the service or an error family, use Tempo to inspect matching traces.

```bash
curl -X POST http://127.0.0.1:8000/README.md \
  -H "Content-Type: application/json" \
  -d '{"command":["curl","-sS","-G","-H","Authorization: Bearer $GRAFANA_TOKEN","$TEMPO_SEARCH_URL","--data-urlencode","q={ resource.service.name = \"gateway\" && span.http.status_code >= 500 }","--data-urlencode","limit=20","--data-urlencode","spss=3"]}'
```

## Required Environment Variables
- `GRAFANA_TOKEN`
- `LOKI_QUERY_RANGE_URL`
- `TEMPO_SEARCH_URL`

## Notes
- `LOKI_QUERY_RANGE_URL` usually looks like `https://<grafana-host>/api/datasources/proxy/uid/<logs-uid>/loki/api/v1/query_range`.
- `TEMPO_SEARCH_URL` usually looks like `https://<grafana-host>/api/datasources/proxy/uid/<tempo-uid>/api/search`.
- Loki `query_range` accepts `query`, `limit`, `since`, `start`, `end`, and `direction`.
- Useful LogQL starting points:
  - `{service_name="gateway"}`
  - `{service_name="gateway"} |= "500"`
  - `{service_name="gateway"} | json | status_code >= 500`
  - `{service_name="gateway"} |= "timeout"`
- Useful TraceQL starting points:
  - `{ resource.service.name = "gateway" }`
  - `{ resource.service.name = "gateway" && span.http.status_code >= 500 }`
  - `{ resource.service.name = "gateway" && name = "GET /health" }`
- This starter stays close to raw Grafana APIs on purpose so agents do not have to learn a wrapper script first.

```component
for name in GRAFANA_TOKEN LOKI_QUERY_RANGE_URL TEMPO_SEARCH_URL; do
  value="$(printenv "$name" 2>/dev/null || true)"
  if [ -n "$value" ]; then
    if [ "$name" = "GRAFANA_TOKEN" ]; then
      echo "$name: set"
    else
      echo "$name: $value"
    fi
  else
    echo "$name: missing"
  fi
done
```
