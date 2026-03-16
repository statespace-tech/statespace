#!/usr/bin/env python3
"""Fetch posts from a subreddit. Outputs JSON.

Usage: fetch.py <subreddit> [sort] [limit]

  subreddit  Subreddit name (without r/)
  sort       hot, top, new, rising (default: hot)
  limit      Number of posts, 1-25 (default: 10)
"""

import json
import sys
import urllib.request

SORTS = {"hot", "top", "new", "rising"}
USER_AGENT = "statespace-demo/1.0"


def fetch(subreddit, sort="hot", limit=10):
    url = f"https://www.reddit.com/r/{subreddit}/{sort}.json?limit={limit}&t=week"
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=10) as resp:
        data = json.loads(resp.read())

    posts = []
    for child in data["data"]["children"]:
        p = child["data"]
        posts.append({
            "title": p["title"],
            "score": p["score"],
            "num_comments": p["num_comments"],
            "author": p["author"],
            "url": f"https://reddit.com{p['permalink']}",
            "created_utc": p["created_utc"],
            "selftext": p.get("selftext", "")[:200],
        })

    return {
        "subreddit": subreddit,
        "sort": sort,
        "count": len(posts),
        "posts": posts,
    }


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(1)

    subreddit = sys.argv[1]
    sort = sys.argv[2] if len(sys.argv) > 2 else "hot"
    limit = int(sys.argv[3]) if len(sys.argv) > 3 else 10

    if sort not in SORTS:
        print(f"Invalid sort: {sort}. Choose from: {', '.join(SORTS)}", file=sys.stderr)
        sys.exit(1)

    limit = max(1, min(25, limit))

    try:
        result = fetch(subreddit, sort, limit)
        json.dump(result, sys.stdout, indent=2)
        print()
    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout)
        sys.exit(1)
