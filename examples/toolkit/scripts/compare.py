#!/usr/bin/env python3
"""Compare two subreddits side by side.

Usage: compare.py <subreddit1> <subreddit2> [sort] [limit]

  sort   hot, top, new, rising (default: hot)
  limit  Posts per subreddit, 1-25 (default: 10)
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
        })
    return posts


def stats(posts):
    scores = [p["score"] for p in posts]
    comments = [p["num_comments"] for p in posts]
    authors = set(p["author"] for p in posts)
    return {
        "posts": len(posts),
        "total_score": sum(scores),
        "avg_score": sum(scores) // max(len(scores), 1),
        "max_score": max(scores, default=0),
        "total_comments": sum(comments),
        "avg_comments": sum(comments) // max(len(comments), 1),
        "unique_authors": len(authors),
        "authors": list(authors),
    }


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(1)

    sub1, sub2 = sys.argv[1], sys.argv[2]
    sort = sys.argv[3] if len(sys.argv) > 3 else "hot"

    if sort not in SORTS:
        print(f"Invalid sort: {sort}. Choose from: {', '.join(SORTS)}", file=sys.stderr)
        sys.exit(1)

    try:
        limit = int(sys.argv[4]) if len(sys.argv) > 4 else 10
    except ValueError:
        json.dump({"error": f"Invalid limit: {sys.argv[4]}"}, sys.stdout)
        sys.exit(1)

    limit = max(1, min(25, limit))

    try:
        posts1 = fetch(sub1, sort, limit)
        posts2 = fetch(sub2, sort, limit)

        s1 = stats(posts1)
        s2 = stats(posts2)

        shared_authors = set(s1["authors"]) & set(s2["authors"])

        result = {
            "subreddits": [sub1, sub2],
            "sort": sort,
            sub1: s1,
            sub2: s2,
            "shared_authors": list(shared_authors),
            "engagement_ratio": round(
                s1["total_score"] / max(s2["total_score"], 1), 2
            ),
        }

        # Remove author lists from per-sub stats (redundant)
        del result[sub1]["authors"]
        del result[sub2]["authors"]

        json.dump(result, sys.stdout, indent=2)
        print()
    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout)
        sys.exit(1)
