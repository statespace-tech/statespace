---
tools:
  - [python3, scripts/fetch.py, { }, ;]
  - [python3, scripts/wordcloud.py, { }, ;]
  - [python3, scripts/compare.py, { }, ;]
---

# Reddit Toolkit

Query Reddit's public API. No authentication required.

## Tools

**fetch.py** `<subreddit> [sort] [limit]` — Get posts with titles, scores, comments, and authors.

**wordcloud.py** `<subreddit> [sort] [limit]` — Get the most frequent keywords across post titles.

**compare.py** `<sub1> <sub2> [sort] [limit]` — Compare two subreddits: engagement, activity, shared authors.

`sort`: hot, top, new, rising (default: hot). `limit`: 1–25 (default: 10).

## Questions you can ask

- What's trending on r/programming?
- What are the buzzwords on r/machinelearning right now?
- How does r/python compare to r/rust in engagement?
