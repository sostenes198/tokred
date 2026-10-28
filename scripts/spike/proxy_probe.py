"""Throwaway spike: a pass-through proxy that logs what Claude Code sends. NOT part of tokred.

Never logs credential values or message contents: only header names, x-claude-code-* values,
anthropic-beta/version, and structural facts about the body (model, stream, counts) and usage.
Env: SPIKE_LOG (log path), SPIKE_PORT (default 0 = random), SPIKE_REWRITE="from:to" (probe: rewrite the model field).
"""
import http.client, json, os, ssl, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = "api.anthropic.com"
LOG = os.environ.get("SPIKE_LOG", "/tmp/tokred-spike/log.jsonl")
HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailers",
       "transfer-encoding", "upgrade", "host", "content-length", "accept-encoding"}
SECRET = {"x-api-key", "authorization", "cookie", "proxy-authorization"}
REWRITE = os.environ.get("SPIKE_REWRITE", "")
lock = threading.Lock()

def log(rec):
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(rec) + "\n")

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass

    def do_POST(self): self.forward()
    def do_GET(self): self.forward()

    def forward(self):
        t0 = time.time()
        n = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(n) if n else b""
        info = {"ts": int(t0), "method": self.command, "path": self.path}
        info["header_names"] = sorted({k.lower() for k in self.headers.keys()})
        info["auth_headers_present"] = sorted(k for k in info["header_names"] if k in SECRET)
        info["claude_code_headers"] = {k.lower(): v for k, v in self.headers.items() if k.lower().startswith("x-claude-code-")}
        info["anthropic_beta"] = self.headers.get("anthropic-beta")
        info["anthropic-version"] = self.headers.get("anthropic-version")
        out_body = body
        if self.path.startswith("/v1/messages") and body:
            try:
                j = json.loads(body)
                info["model"] = j.get("model")
                info["stream"] = j.get("stream")
                info["tools"] = len(j.get("tools") or [])
                info["messages"] = len(j.get("messages") or [])
                sysb = j.get("system")
                info["system_blocks"] = len(sysb) if isinstance(sysb, list) else (1 if sysb else 0)
                info["thinking"] = j.get("thinking")
                info["output_config"] = j.get("output_config")
                info["max_tokens"] = j.get("max_tokens")
                info["body_keys"] = sorted(j.keys())
                if REWRITE and ":" in REWRITE:
                    src, dst = REWRITE.split(":", 1)
                    agent_only = os.environ.get("SPIKE_AGENT_ONLY") == "1"
                    if src and src in str(j.get("model")) and (not agent_only or any(k.lower() == "x-claude-code-agent-id" for k in self.headers.keys())):
                        j["model"] = dst
                        info["rewritten_to"] = dst
                        out_body = json.dumps(j).encode()
            except Exception as e:
                info["body_parse_error"] = type(e).__name__
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
        headers["Host"] = UPSTREAM
        headers["Content-Length"] = str(len(out_body))
        ctx = ssl.create_default_context()
        conn = http.client.HTTPSConnection(UPSTREAM, context=ctx, timeout=300)
        try:
            conn.request(self.command, self.path, body=out_body or None, headers=headers)
            resp = conn.getresponse()
        except Exception as e:
            info["upstream_error"] = type(e).__name__
            log(info)
            self.send_response(502); self.send_header("content-length", "0"); self.end_headers()
            return
        info["status"] = resp.status
        info["ttfb_ms"] = int((time.time() - t0) * 1000)
        self.send_response(resp.status)
        keep = {}
        for k, v in resp.getheaders():
            lk = k.lower()
            if lk in HOP: continue
            self.send_header(k, v)
            if lk in ("content-type", "anthropic-ratelimit-requests-remaining", "request-id", "retry-after"):
                keep[lk] = v
        info["resp_headers"] = keep
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        buf = b""
        while True:
            chunk = resp.read1(65536)
            if not chunk: break
            self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n"); self.wfile.flush()
            buf += chunk
        self.wfile.write(b"0\r\n\r\n"); self.wfile.flush()
        info["total_ms"] = int((time.time() - t0) * 1000)
        info["resp_bytes"] = len(buf)
        text = buf.decode("utf-8", "replace")
        usage, model = {}, None
        if "text/event-stream" in keep.get("content-type", ""):
            for line in text.splitlines():
                if line.startswith("data:"):
                    try:
                        d = json.loads(line[5:])
                    except Exception: continue
                    if d.get("type") == "message_start":
                        m = d.get("message", {}); model = m.get("model"); usage.update(m.get("usage") or {})
                    elif d.get("type") == "message_delta":
                        usage.update(d.get("usage") or {})
        else:
            try:
                d = json.loads(text); model = d.get("model"); usage = d.get("usage") or {}
                if resp.status >= 400: info["error_type"] = (d.get("error") or {}).get("type"); info["error_message"] = (d.get("error") or {}).get("message", "")[:200]
            except Exception: pass
        info["resp_model"] = model
        info["usage"] = {k: v for k, v in usage.items() if isinstance(v, (int, float))}
        log(info)
        conn.close()

if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("SPIKE_PORT", "0"))), Handler)
    print(srv.server_address[1], flush=True)
    srv.serve_forever()
