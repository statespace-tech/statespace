---
tools:
  - [grep]
---

```component
echo "There are $(ls ./logs/* | wc -l | xargs) files under ./logs"
```

Use `grep` to search through them.

## Questions you can ask

- Were there any database connection failures?
- Which services logged authentication errors?
- What deployment happened most recently and did it succeed?