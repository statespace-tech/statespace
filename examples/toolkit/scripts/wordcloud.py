#!/usr/bin/env python3
"""Extract top keywords from a subreddit's posts.

Usage: wordcloud.py <subreddit> [sort] [limit]

  subreddit  Subreddit name (without r/)
  sort       hot, top, new, rising (default: hot)
  limit      Number of top words to return (default: 20)
"""

import json
import re
import sys
import urllib.request
from collections import Counter

USER_AGENT = "statespace-demo/1.0"
SORTS = ["hot", "new", "top", "controversial", "rising"]

STOP_WORDS = {
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of",
    "with", "by", "from", "is", "it", "its", "are", "was", "were", "be", "been",
    "has", "have", "had", "do", "does", "did", "will", "would", "could", "should",
    "not", "no", "so", "if", "as", "than", "that", "this", "what", "which", "who",
    "how", "when", "where", "why", "all", "can", "just", "about", "up", "out",
    "my", "your", "i", "you", "we", "they", "he", "she", "me", "us", "them",
    "any", "some", "more", "most", "very", "too", "also", "like", "into", "over",
    "after", "before", "between", "through", "during", "without", "again", "there",
    "here", "then", "now", "get", "got", "one", "two", "new", "first", "even",
    "way", "may", "much", "many", "each", "made", "make", "still", "own", "really",
    "don", "doesn", "didn", "won", "isn", "aren", "don't", "doesn't", "didn't",
    "it's", "i'm", "i've", "he's", "she's", "we're", "they're", "that's",
    "there's", "here's", "what's", "who's", "let's", "can't", "won't", "shouldn't",
    "been", "being", "going", "know", "think", "want", "need", "use", "using",
    "used", "every", "other", "only", "same", "well", "back", "good", "best",
    "better", "right", "thing", "things", "something", "anything", "nothing",
    "someone", "anyone", "everyone", "people", "time", "year", "years", "day",
    "long", "great", "lot", "amp", "http", "https", "www", "com",
}


def fetch_posts(subreddit, sort, count=25):
    url = f"https://www.reddit.com/r/{subreddit}/{sort}.json?limit={count}&t=week"
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=10) as resp:
        data = json.loads(resp.read())
    return [child["data"] for child in data["data"]["children"]]


def extract_words(posts):
    texts = []
    for p in posts:
        texts.append(p.get("title", ""))
        texts.append(p.get("selftext", "")[:200])

    words = []
    for text in texts:
        tokens = re.findall(r"[a-z][a-z'-]*[a-z]|[a-z]", text.lower())
        words.extend(t for t in tokens if t not in STOP_WORDS and len(t) > 2)

    return words


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(1)

    subreddit = sys.argv[1]
    sort = sys.argv[2] if len(sys.argv) > 2 else "hot"
    try:
        limit = int(sys.argv[3]) if len(sys.argv) > 3 else 20
    except ValueError:
        json.dump({"error": f"Invalid limit: {sys.argv[3]}"}, sys.stdout)
        sys.exit(1)

    if sort not in SORTS:
        print(f"Invalid sort: {sort}. Choose from: {', '.join(SORTS)}", file=sys.stderr)
        sys.exit(1)

    try:
        posts = fetch_posts(subreddit, sort)
        words = extract_words(posts)
        counts = Counter(words).most_common(limit)

        result = {
            "subreddit": subreddit,
            "total_words": len(words),
            "unique_words": len(set(words)),
            "top_words": [{"word": w, "count": c} for w, c in counts],
        }

        json.dump(result, sys.stdout, indent=2)
        print()
    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout)
        sys.exit(1)
