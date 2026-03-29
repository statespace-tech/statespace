---
tools:
  - [mongosh, $MONGODB_URI, --eval, { regex: "^db\\.\\w+\\.(find|findOne|aggregate|count|distinct)\\(" }]
---

# Instructions
- List collections with `db.getCollectionNames()`
- Sample a document with `db.<collection>.findOne()` to understand the schema before querying
- See [MongoDB documentation](https://www.mongodb.com/docs/) for reference
