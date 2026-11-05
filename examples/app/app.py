"""A tiny app for trying `ssp proxy`: it says which experiment and arm served
the request, and shows the labels that an arm gets. Each response names the
event `visit` in a header, which an event rule over `response.headers` can
count."""

import json
import os
import socket
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# The variables that label an arm. Live has none of them.
LABELS = ["SSP_EXPERIMENT", "SSP_ARM", "OTEL_RESOURCE_ATTRIBUTES"]


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(
            {
                # The proxy names the experiment and the arm. Live gets none.
                "experiment": self.headers.get("ssp-experiment"),
                "arm": self.headers.get("ssp-arm"),
                "labels": {name: os.environ.get(name) for name in LABELS},
                "greeting": os.environ.get("GREETING", "hello"),
                "shared": os.environ.get("SHARED"),
                # The app's own credential. The proxy never changes it.
                "authorization": self.headers.get("authorization"),
                # Names only: the proxy never passes its own settings on.
                "statespace_variables": sorted(
                    name for name in os.environ if name.startswith("STATESPACE_")
                ),
            }
        ).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("x-statespace-event", "visit")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        print(f"{self.command} {self.path}", file=sys.stderr, flush=True)


class DualStackServer(ThreadingHTTPServer):
    # Listen on IPv6 and IPv4. Fly.io's private network is IPv6 only.
    address_family = socket.AF_INET6

    def server_bind(self):
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        super().server_bind()


port = int(sys.argv[1] if len(sys.argv) > 1 else os.environ.get("PORT", "8000"))
try:
    server = DualStackServer(("::", port), Handler)
except OSError:
    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
server.serve_forever()
