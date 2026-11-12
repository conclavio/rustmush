#!/usr/bin/env python3
"""llm-stub.py: an OpenAI-shaped chat endpoint on loopback that answers every
request with one short line.

scripts/launch-bringup-acceptance.sh (LLM=stub) points the scratch instance's
providers at it, so the bring-up's LLM check can be shown to PASS without a
key and without spending a token: the check reads `@llm/log` for an `ok` row
on an `npc_dialogue_*` route and on `tacsim_deliberate`, and an `ok` row is
written when a provider answers. What the answer says is not under test.

Usage: llm-stub.py <port>
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Stub(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        try:
            request = json.loads(self.rfile.read(length) or b"{}")
        except ValueError:
            request = {}
        body = json.dumps({
            "id": "stub",
            "object": "chat.completion",
            "created": 0,
            "model": request.get("model", "stub"),
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "I keep the lamps lit here."},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19},
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        print(f"{self.path} model={request.get('model')} tools={len(request.get('tools') or [])}",
              flush=True)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Stub).serve_forever()
