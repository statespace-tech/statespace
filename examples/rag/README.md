---
tools:
  - [grep]
---

```component
echo "There are $(find ./logs -maxdepth 1 -type f 2>/dev/null | wc -l | xargs) files under ./logs"
```

Use `grep` to search through them.

## Questions you can ask

- Were there any database connection failures?
- Which services logged authentication errors?
- What deployment happened most recently and did it succeed?