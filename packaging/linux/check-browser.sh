#!/usr/bin/env bash
# Starts the browser program (camallo-browser) on a page with a video, served from this machine, and checks that
# it reports the video: the page, its title and the file itself with its size. Needs a display (run it under
# xvfb-run), python3 and, unless CAMALLO_CHECK_VIDEO names a video file, ffmpeg to make one.
#
#   xvfb-run -a packaging/linux/check-browser.sh target/release/camallo-browser
set -euo pipefail

browser=${1:?usage: check-browser.sh <camallo-browser>}
browser=$(cd "$(dirname "$browser")" && pwd)/$(basename "$browser")
work=$(mktemp -d)
server=
trap '[ -n "$server" ] && kill "$server" 2>/dev/null; rm -rf "$work"' EXIT

if [ -n "${CAMALLO_CHECK_VIDEO:-}" ]; then
  cp "$CAMALLO_CHECK_VIDEO" "$work/sample.webm"
else
  ffmpeg -loglevel error -f lavfi -i testsrc2=size=480x270:rate=25 -f lavfi -i sine=frequency=440:duration=6 \
    -t 6 -c:v libvpx -b:v 600k -c:a libvorbis "$work/sample.webm"
fi
size=$(stat -c %s "$work/sample.webm")

cat >"$work/page.html" <<'HTML'
<!doctype html><html><head><title>Camallo test page</title></head>
<body><h1>Test video</h1><video src="sample.webm" autoplay muted loop controls width="480"></video></body></html>
HTML

# A server that answers byte ranges, as the video players of real sites expect.
cat >"$work/serve.py" <<'PY'
import http.server, os, re, sys
class H(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
    def send_head(self):
        path = self.translate_path(self.path)
        rng = self.headers.get('Range')
        if os.path.isfile(path) and rng:
            m = re.match(r'bytes=(\d+)-(\d*)', rng)
            total = os.path.getsize(path)
            start = int(m.group(1)); end = min(int(m.group(2)) if m.group(2) else total - 1, total - 1)
            f = open(path, 'rb'); f.seek(start)
            self.send_response(206)
            self.send_header('Content-Type', self.guess_type(path))
            self.send_header('Content-Range', 'bytes %d-%d/%d' % (start, end, total))
            self.send_header('Content-Length', str(end - start + 1))
            self.send_header('Accept-Ranges', 'bytes')
            self.end_headers()
            self._left = end - start + 1
            return f
        return super().send_head()
    def copyfile(self, src, dst):
        n = getattr(self, '_left', None)
        if n is None: return super().copyfile(src, dst)
        while n > 0:
            b = src.read(min(65536, n))
            if not b: break
            dst.write(b); n -= len(b)
        self._left = None
os.chdir(sys.argv[1])
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[2])), H).serve_forever()
PY
port=$((20000 + RANDOM % 20000))
python3 "$work/serve.py" "$work" "$port" &
server=$!
sleep 1

# No GPU and no user namespaces are to be had on a build machine.
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
timeout "${CAMALLO_CHECK_SECONDS:-25}" "$browser" "http://127.0.0.1:$port/page.html" "$work/profile" >"$work/out.txt" 2>"$work/err.txt" || true

grep '^{' "$work/out.txt" >"$work/lines.txt" || true
echo "the browser reported:"; cut -c1-160 "$work/lines.txt"
fail() { echo "FAILED: $1"; tail -5 "$work/err.txt"; exit 1; }
grep -q '"kind":"page"' "$work/lines.txt" || fail "the page was not reported"
grep -q '"title":"Camallo test page"' "$work/lines.txt" || fail "the title was not reported"
grep -q "\"kind\":\"media\".*\"size\":$size,.*sample.webm" "$work/lines.txt" || fail "the video was not reported with its size ($size)"
echo "OK"
