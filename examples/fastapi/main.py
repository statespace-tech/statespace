"""A small shop for trying Statespace with FastAPI.

Run it behind `ssp proxy`. A purchase names itself in the response header
`x-statespace-event`. The `[[event]]` rules in statespace.toml count signups and
purchases for each arm of an experiment. An arm is a commit of this file, built
into an image in CI.
"""

import logging

from fastapi import FastAPI, HTTPException, Response
from pydantic import BaseModel

# The change an arm tests. A commit might lower the price or rewrite checkout.
PRICE = 40

logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
log = logging.getLogger("shop")
app = FastAPI()


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
def signup(body: Signup):
    log.info("signup")
    return {"welcome": body.email}


@app.post("/checkout")
def checkout(body: Order, response: Response):
    if body.quantity < 1:
        raise HTTPException(422, "quantity must be at least 1")
    amount = PRICE * body.quantity
    log.info("charged %s", amount)
    # The rule of the event `purchase` matches this header.
    response.headers["x-statespace-event"] = "purchase"
    return {"amount": amount}
