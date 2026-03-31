---
tools:
  - [mongosh, $MONGODB_URI, --eval, { regex: "^db\\.\\w+\\.(find|findOne|aggregate|count|distinct)\\(" }]
---

# Instructions
- Explore the schema to understand the data model
- Follow the user's instructions and answer their questions
- Reference [documentation](https://www.mongodb.com/docs/) as needed
