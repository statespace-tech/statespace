---
tools:
  - [curl, -s, -H, "Content-Type: application/json", -H, "api-key: $QDRANT_API_KEY", { }]
---

# Instructions
- List collections with `GET $QDRANT_URL/collections` before querying
- Points have an `id`, a `vector`, and an optional `payload` (arbitrary JSON)
- Always specify `limit` in search requests; distance options: `Cosine`, `Euclid`, `Dot`, `Manhattan`
- See [Qdrant documentation](https://qdrant.tech/documentation/) for reference
