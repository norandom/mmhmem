"""Worker fixture component tests and independent HTTPS/DNS acceptance client."""

import argparse
import hashlib
import json
import ssl
import unittest
import urllib.error
import urllib.request

from worker_fixture import response_marker


class WorkerFixtureTests(unittest.TestCase):
    def test_marker_binds_validation_and_exact_request(self):
        first = response_marker("validation-one", b"request one")
        self.assertNotEqual(first, response_marker("validation-two", b"request one"))
        self.assertNotEqual(first, response_marker("validation-one", b"request two"))
        self.assertIn(hashlib.sha256(b"request one").hexdigest()[:16], first)


def request(url, body=None):
    payload = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    # Default SSL context reads the runtime's normal CA trust store.
    with urllib.request.urlopen(req, timeout=10) as response:
        return json.load(response)


def verify(endpoint, validation_id, trusted):
    body = {"model": validation_id, "temperature": 0.0,
            "messages": [{"role": "user", "content": "independent hostname TLS probe"}]}
    if not trusted:
        try:
            request(endpoint + "/v1/chat/completions", body)
        except urllib.error.URLError as error:
            if not isinstance(error.reason, ssl.SSLCertVerificationError):
                raise AssertionError(f"expected certificate rejection, got {error!r}") from error
            print(json.dumps({"status": "passed", "case": "untrusted", "certificate_rejected": True}))
            return
        raise AssertionError("untrusted worker certificate was accepted")
    before = request(endpoint + "/receipts")
    response = request(endpoint + "/v1/chat/completions", body)
    digest = hashlib.sha256(json.dumps(body).encode()).hexdigest()
    marker = response_marker(validation_id, json.dumps(body).encode())
    assert response["choices"][0]["message"]["content"] == marker
    receipts = request(endpoint + "/receipts")
    assert receipts["accepted_count"] == before["accepted_count"] + 1, receipts
    assert receipts["requests"][-1]["request_sha256"] == digest, receipts
    assert receipts["requests"][-1]["response_marker"] == marker, receipts
    assert response["usage"] == {"prompt_tokens": 500, "completion_tokens": 30}
    assert receipts["validation_id"] == validation_id, receipts
    print(json.dumps({"status": "passed", "case": "trusted", "receipt": receipts["requests"][-1],
                      "tls_error_count": len(receipts["tls_errors"])}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--validation-id", required=True)
    parser.add_argument("--mode", choices=("trusted", "untrusted", "receipts"), required=True)
    args = parser.parse_args()
    if args.mode == "receipts":
        evidence = request(args.endpoint + "/receipts")
        assert evidence["accepted_count"] == 1, evidence
        assert evidence["tls_errors"], evidence
        print(json.dumps(evidence))
    else:
        verify(args.endpoint, args.validation_id, args.mode == "trusted")
