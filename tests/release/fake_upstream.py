#!/usr/bin/env python3
"""A fake Anthropic API for the release smoke test (standard library only, Python 3.7+).

    python3 -I fake_upstream.py [--port N] [--port-file FILE] [--delay-ms 80] [--chunks 5]

Answers `POST /v1/messages` the way the real API does, without any model behind it:

  * `"stream": true`  -> a Server-Sent-Events message that is STREAMED slowly: `message_start`, one
    `content_block_delta` per word with `--delay-ms` between them, `message_delta` (stop reason
    `end_turn`, output tokens) and `message_stop`. A proxy that buffers the whole reply is caught by
    the time between the first byte and the last.
  * otherwise        -> the same message as one JSON document (the non-streaming mode).

Every reply has a UNIQUE message id (`msg_fake_<n>`), the model that was asked for, 120 input and
15 output tokens, and the text `fake-reply: <first words of the last user message>` so a test can
tell which request a reply belongs to. The reply carries `x-fake-upstream: 1`.

Other paths: `GET /__fake/health` answers 200; anything else is a 404 JSON error.

What it never does: it keeps no request body, header or credential, not in memory and not on disk,
and prints one line per request (method, path, status) to stderr. It listens on 127.0.0.1 only.

Environment / options:
  --port N          port to listen on (0 or absent: a free one is chosen)
  --port-file FILE  write the chosen port there once the socket listens (how a shell learns it)
  --delay-ms N      pause between two streamed chunks (default 80)
  --chunks N        at least this many streamed text chunks (default 5; the reply is padded)
"""

import argparse
import itertools
import json
import re
import socketserver
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer



class QuickServer(ThreadingHTTPServer):
    """A server that does not ask DNS for its own name.

    `HTTPServer.server_bind` calls `socket.getfqdn`, a reverse lookup that can take tens of seconds on
    some CI runners (macOS), and the name is never used here.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


IDS = itertools.count(1)
ID_LOCK = threading.Lock()
SETTINGS = {"delay": 0.08, "chunks": 5}


def next_id():
    with ID_LOCK:
        return "msg_fake_%d" % next(IDS)


def last_user_text(body):
    """The first words of the last user message, reduced to a harmless character set."""
    messages = body.get("messages") if isinstance(body, dict) else None
    if not isinstance(messages, list):
        return "none"
    for message in reversed(messages):
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, list):
            content = " ".join(
                block.get("text", "") for block in content if isinstance(block, dict)
            )
        if isinstance(content, str):
            words = re.sub(r"[^A-Za-z0-9_ -]", "", content).split()[:4]
            return " ".join(words) or "empty"
    return "none"


def reply_words(body):
    words = ("fake-reply: " + last_user_text(body)).split(" ")
    while len(words) < SETTINGS["chunks"]:
        words.append("ok")
    return words


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "fake-upstream"

    def log_message(self, fmt, *args):  # nothing of the request is logged
        pass

    def _log(self, status):
        sys.stderr.write("%s %s -> %d\n" % (self.command, self.path.split("?")[0], status))
        sys.stderr.flush()

    def _json(self, status, document):
        data = json.dumps(document).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.send_header("x-fake-upstream", "1")
        self.end_headers()
        self.wfile.write(data)
        self._log(status)

    def do_GET(self):
        if self.path.split("?")[0] == "/__fake/health":
            self._json(200, {"ok": True})
        else:
            self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such path"}})

    def do_POST(self):
        length = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(length) if length else b""
        if self.path.split("?")[0] != "/v1/messages":
            self._json(404, {"type": "error", "error": {"type": "not_found_error", "message": "no such path"}})
            return
        try:
            body = json.loads(raw.decode("utf-8") or "{}")
        except ValueError:
            self._json(400, {"type": "error", "error": {"type": "invalid_request_error", "message": "bad json"}})
            return
        model = body.get("model", "claude-fake") if isinstance(body, dict) else "claude-fake"
        words = reply_words(body)
        text = " ".join(words)
        message_id = next_id()
        usage_in = {"input_tokens": 120, "output_tokens": 1,
                    "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
        if isinstance(body, dict) and body.get("stream"):
            self._stream(message_id, model, words, usage_in)
        else:
            self._json(200, {
                "id": message_id, "type": "message", "role": "assistant", "model": model,
                "content": [{"type": "text", "text": text}],
                "stop_reason": "end_turn", "stop_sequence": None,
                "usage": dict(usage_in, output_tokens=15),
            })

    def _stream(self, message_id, model, words, usage_in):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("x-fake-upstream", "1")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()

        def chunk(payload):
            data = payload.encode()
            self.wfile.write(b"%x\r\n" % len(data) + data + b"\r\n")
            self.wfile.flush()

        # Chunked framing around the SSE events, written by hand so each event leaves at once.
        def event(name, data):
            chunk("event: %s\ndata: %s\n\n" % (name, json.dumps(data)))

        event("message_start", {"type": "message_start", "message": {
            "id": message_id, "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": None, "stop_sequence": None, "usage": usage_in}})
        event("content_block_start", {"type": "content_block_start", "index": 0,
                                      "content_block": {"type": "text", "text": ""}})
        event("ping", {"type": "ping"})
        for number, word in enumerate(words):
            time.sleep(SETTINGS["delay"])
            piece = word if number == 0 else " " + word
            event("content_block_delta", {"type": "content_block_delta", "index": 0,
                                          "delta": {"type": "text_delta", "text": piece}})
        event("content_block_stop", {"type": "content_block_stop", "index": 0})
        event("message_delta", {"type": "message_delta",
                                "delta": {"stop_reason": "end_turn", "stop_sequence": None},
                                "usage": {"output_tokens": 15}})
        event("message_stop", {"type": "message_stop"})
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()
        self._log(200)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--port-file")
    parser.add_argument("--delay-ms", type=int, default=80)
    parser.add_argument("--chunks", type=int, default=5)
    args = parser.parse_args()
    SETTINGS["delay"] = args.delay_ms / 1000.0
    SETTINGS["chunks"] = max(1, args.chunks)
    server = QuickServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    port = server.server_address[1]
    if args.port_file:
        # Written last and atomically, so a reader never sees half a number.
        tmp = args.port_file + ".tmp"
        with open(tmp, "w") as handle:
            handle.write("%d\n" % port)
        import os
        os.replace(tmp, args.port_file)
    sys.stderr.write("fake upstream listening on 127.0.0.1:%d\n" % port)
    sys.stderr.flush()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
