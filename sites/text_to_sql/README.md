---
tools:
  - [sqlite3, store.db, { regex: "^SELECT\\b.*" }]
---

# E-Commerce Store

```component
sqlite3 store.db "SELECT count(*) FROM customers" | xargs -I{} echo "{} customers"
sqlite3 store.db "SELECT count(*) FROM products" | xargs -I{} echo "{} products"
sqlite3 store.db "SELECT count(*) FROM orders" | xargs -I{} echo "{} orders"
```

Use `sqlite3` to query the database. Only `SELECT` queries are allowed.

## Schema

**customers** — id, name, email, city, country, joined

**products** — id, name, category (Electronics, Furniture), price

**orders** — id, customer_id, product_id, quantity, ordered_at

## Questions you can ask

- Who are the top 3 customers by total spend?
- What's the most popular product category?
- Which countries have customers who ordered in 2025?
