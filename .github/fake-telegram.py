"""A stand-in for Telegram's bot API, for trying an installed Orbite: it
accepts everything and never has anything to pass on."""
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Api(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length") or 0))
        if self.path.endswith("/getUpdates"):
            time.sleep(5)
            result = []
        elif self.path.endswith("/getMe"):
            result = {"id": 1, "is_bot": True, "first_name": "Try", "username": "try_bot"}
        else:
            result = True
        body = json.dumps({"ok": True, "result": result}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST

    def log_message(self, *arguments):
        pass


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Api).serve_forever()
