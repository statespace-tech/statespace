"""A tiny app for trying `ssp proxy`: it says which side served the request,
and prints a JSON line for every subject it serves."""

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        subject = self.headers.get("x-statespace-subject")
        body = json.dumps(
            {
                # The proxy names the side: control or arm. Live gets none.
                "side": self.headers.get("x-statespace-side") or "live",
                "arm": self.headers.get("x-statespace-arm"),
                "subject": subject,
                "greeting": os.environ.get("GREETING", "hello"),
                "shared": os.environ.get("SHARED"),
                # Names only: the proxy never passes its own settings on.
                "statespace_variables": sorted(
                    name for name in os.environ if name.startswith("STATESPACE_")
                ),
            }
        ).encode()
        if subject:
            print(json.dumps({"event": "visit", "path": self.path}), flush=True)
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        print(f"{self.command} {self.path}", file=sys.stderr, flush=True)


port = int(sys.argv[1] if len(sys.argv) > 1 else os.environ.get("PORT", "8000"))
ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()
