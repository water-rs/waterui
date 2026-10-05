#!/usr/bin/env python3
"""Local Maven proxy for CI builds on this VM.

repo.maven.apache.org returns HTTP 429 for this shared egress IP. All Gradle
builds route Maven Central through this proxy, which forwards to Google's
official GCS mirror of Maven Central, falling back to dl.google.com (Android
artifacts) and jitpack.io.

Started by bench.py during `build` when RN is involved (RN autolinked modules
declare their own repositories and cannot be rewritten from an init script).
"""
import http.server
import urllib.request
import urllib.error
import sys

UPSTREAMS = [
    "https://maven-central.storage-download.googleapis.com/maven2",
    "https://dl.google.com/dl/android/maven2",
    "https://www.jitpack.io",
    "https://plugins.gradle.org/m2",
    "https://repo.maven.apache.org/maven2",  # last resort
]


class Handler(http.server.BaseHTTPRequestHandler):
    def _fetch(self, method: str):
        path = self.path
        if path.startswith("/m2/"):
            path = path[3:]
        for base in UPSTREAMS:
            req = urllib.request.Request(base + path, method=method)
            try:
                with urllib.request.urlopen(req, timeout=30) as r:
                    if method == "HEAD":
                        self.send_response(200)
                        for h in ("Content-Type", "Content-Length"):
                            if r.headers.get(h):
                                self.send_header(h, r.headers[h])
                        self.end_headers()
                        return
                    body = r.read()
                    self.send_response(200)
                    self.send_header("Content-Type",
                                     r.headers.get("Content-Type",
                                                   "application/octet-stream"))
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
            except urllib.error.HTTPError as e:
                if e.code in (401, 403, 404):
                    continue
                # upstream failure (e.g. 429): try next
                continue
            except Exception:
                continue
        self.send_response(404)
        self.end_headers()

    def do_GET(self):
        self._fetch("GET")

    def do_HEAD(self):
        self._fetch("HEAD")

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
