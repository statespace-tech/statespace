"""A small shop for trying Statespace with FastAPI.

Run it behind `ssp proxy`. It prints a JSON line for each signup and purchase.
The `[[event]]` rules in statespace.toml count them for each arm of an experiment.
An arm is a commit of this file, built into an image in CI.
"""

import json
import logging
import os

from fastapi import FastAPI, Header, HTTPException
from pydantic import BaseModel

# The change an arm tests. A commit might lower the price or rewrite checkout.
PRICE = 40

logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
log = logging.getLogger("shop")
app = FastAPI()


def event(name: str, subject: str | None, **data: object) -> None:
    """Print one JSON line, which an `[[event]]` rule can match."""
    print(json.dumps({"event": name, "subject": subject, **data}), flush=True)


class Signup(BaseModel):
    email: str


class Order(BaseModel):
    quantity: int = 1


@app.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}


@app.get("/")
def home() -> dict[str, object]:
    return {"price": PRICE}


@app.post("/signup")
def signup(body: Signup, subject: str | None = Header(None, alias="x-statespace-subject")):
    log.info("signup for %s", body.email)
    event("signup", subject)
    return {"welcome": body.email}


@app.post("/checkout")
def checkout(body: Order, subject: str | None = Header(None, alias="x-statespace-subject")):
    if body.quantity < 1:
        raise HTTPException(422, "quantity must be at least 1")
    amount = PRICE * body.quantity
    # Logs can hold secrets. `redact` in statespace.toml removes them before they leave.
    log.info("charged %s with key %s", amount, os.environ.get("PAYMENT_KEY", "sk_test_local"))
    event("purchase", subject, amount=amount, quantity=body.quantity)
    return {"amount": amount}
