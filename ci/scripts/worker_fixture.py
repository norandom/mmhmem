"""Disposable HTTPS chat-completions fixture with correlated public receipts."""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import ssl
import sys
import threading

from release_support import validate_identity


def response_marker(validation_id, payload):
    digest = hashlib.sha256(payload).hexdigest()[:16]
    return f"- fixture_{validation_id}_{digest} [L1-L200] verified HTTPS worker"


class WorkerServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, certificate, key, validation_id):
        validate_identity("0" * 40, validation_id)
        self.validation_id = validation_id
        self.receipts, self.tls_errors = [], []
        self.evidence_lock = threading.Lock()
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.minimum_version = ssl.TLSVersion.TLSv1_2
        self.context.load_cert_chain(certificate, key)
        super().__init__(address, WorkerHandler)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(5)
        try:
            return self.context.wrap_socket(connection, server_side=True), address
        except (ssl.SSLError, OSError) as error:
            connection.close()
            # Keep bounded public diagnostics, never certificate private material.
            with self.evidence_lock:
                self.tls_errors = (self.tls_errors + [str(error)])[-100:]
            print(f"worker TLS rejected: {error}", file=sys.stderr, flush=True)
            raise

    def evidence(self):
        with self.evidence_lock:
            return {"validation_id": self.validation_id, "accepted_count": len(self.receipts),
                    "requests": list(self.receipts), "tls_errors": list(self.tls_errors)}


class WorkerHandler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if self.path == "/receipts":
            self.reply(200, self.server.evidence())
        elif self.path == "/health":
            self.reply(200, {"status": "ok"})
        else:
            self.reply(404, {"error": "unknown fixture endpoint"})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.reply(404, {"error": "unknown fixture endpoint"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 1024 * 1024:
                raise ValueError("request length must be between 1 and 1048576 bytes")
            payload = self.rfile.read(length)
            body = json.loads(payload)
            if not isinstance(body.get("messages"), list) or not body.get("model"):
                raise ValueError("expected chat-completions model and messages")
        except (ValueError, AttributeError) as error:
            self.reply(400, {"error": str(error)})
            return
        marker = response_marker(self.server.validation_id, payload)
        receipt = {"validation_id": self.server.validation_id,
                   "request_sha256": hashlib.sha256(payload).hexdigest(), "response_marker": marker}
        with self.server.evidence_lock:
            self.server.receipts.append(receipt)
        self.reply(200, {"choices": [{"message": {"role": "assistant", "content": marker}}],
                         "usage": {"prompt_tokens": 500, "completion_tokens": 30}})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--certificate", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--validation-id", required=True)
    parser.add_argument("--port", type=int, default=8443)
    args = parser.parse_args()
    with WorkerServer(("0.0.0.0", args.port), args.certificate, args.key, args.validation_id) as server:
        server.serve_forever(poll_interval=0.1)


if __name__ == "__main__":
    main()
