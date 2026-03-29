---
tools:
  - [redis-cli, -h, $REDIS_HOST, -p, $REDIS_PORT, -a, $REDIS_PASSWORD, { }]
---

# Instructions
- Explore key patterns with `SCAN 0 MATCH <pattern> COUNT 100`
- Discover search indexes with `FT._LIST` and inspect them with `FT.INFO <index>`
- Vector similarity search requires a RediSearch index with a VECTOR field
- See [Redis documentation](https://redis.io/docs/) for reference
