---
tools:
  - [grep, -r, -i, { }, ., ;]
  - [cat, { }, ;]
---

# Instructions

- Use `grep` to search for keywords, error messages, or patterns across files
- Use `cat` to read a specific file in full

```component
echo "$(find . -type f | wc -l | xargs) files available"
```
