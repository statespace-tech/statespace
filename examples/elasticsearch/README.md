---
tools:
  - [curl, -s, -u, "$ES_USER:$ES_PASSWORD", -H, "Content-Type: application/json", { }]
---

# Instructions
- List indexes with `GET $ES_URL/_cat/indices?v` and inspect field types with `GET $ES_URL/<index>/_mapping`
- For vector search, the field must be mapped as `dense_vector`; use the top-level `knn` key (Elasticsearch 8+)
- Pass the full URL and any `-X`/`-d` flags as additional arguments after the fixed flags
- See [Elasticsearch documentation](https://www.elastic.co/docs/) for reference
