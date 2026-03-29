---
tools:
  - [curl, -s, -H, "Content-Type: application/json", -H, "Authorization: Bearer $WEAVIATE_API_KEY", { }]
---

# Instructions
- Inspect the schema with `GET $WEAVIATE_URL/v1/schema` before querying
- Collections are called "classes" and are capitalized by convention, e.g. `Document`
- GraphQL is the primary query interface — POST to `/v1/graphql` with a `query` field
- See [Weaviate documentation](https://weaviate.io/developers/weaviate) for reference
