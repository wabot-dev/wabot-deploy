#!/bin/sh
#
# What the edge costs, against nginx doing the same job: the same requests
# sent straight to a container, through nginx, and through the edge, from
# the node itself.
#
#   scripts/bench-proxy.sh <hostname> <container ip:port> [seconds] [big-path]
#
# Run it on the node, as root, with `apk add nghttp2 wrk nginx openssl`.
# On the VM:
#
#   limactl copy scripts/bench-proxy.sh wabot-dev:/tmp/bench-proxy.sh
#   scripts/vm.sh root sh /tmp/bench-proxy.sh bench.wabot.test 10.42.1.5:80
#
# From the node rather than from outside, because the question is what
# the proxy costs and not what the path to the node costs — and on the VM
# Lima's port forwarder is slower than the edge, so a run from the Mac
# measures the forwarder.
#
# nginx is started here, from the configuration below, and stopped on the
# way out, so the baseline is part of the benchmark rather than something
# a machine happens to have. It is set up to do what the edge does: two
# workers for the edge's two runtime threads, TLS with an ECDSA P-256 key
# like the node's own, keep-alive to the container, the same forwarded
# headers, and no access log.
#
# Buffering is nginx's default, which measured best or level with every
# variant tried (off at 4k, 16k, 64k and 256k; on with 32 × 64k). Off at
# its default 4k read halved 1 MB throughput, which would have been a
# baseline set up to lose. `keepalive_requests` is raised because at its
# default of 1,000 nginx sends GOAWAY and h2load stops using the
# connection — the edge has no such limit, and h2 read 500 req/s.
#
# The generator shares the node's CPUs, which is why every line reports
# the proxy's own CPU time per request: requests per second depend on who
# else is running, CPU per request does not.
#
# Latency is h2load's. wrk's histogram cannot be believed over TLS here:
# at one connection it reported a 5.9 ms mean for 20,113 sequential
# requests in 5 s, which is 119 s of waiting. wrk is kept for the one
# thing h2load cannot do, a new connection per request, and its latency
# column is left empty because it starts the clock after the handshake.
set -eu

HOST=${1:?hostname the edge routes}
UPSTREAM=${2:?container address, ip:port}
SECS=${3:-10}
BIG=${4:-/1m.bin}
NGINX_PORT=${NGINX_PORT:-8443}

for tool in h2load wrk nginx openssl; do
  command -v "$tool" >/dev/null || {
    echo "needs $tool: apk add nghttp2 wrk nginx openssl" >&2
    exit 1
  }
done

EDGE=$(pgrep -o -f '^/usr/local/bin/wabot-deploy .*serve$') || {
  echo "no wabot-deploy serve process" >&2
  exit 1
}
HZ=$(getconf CLK_TCK)

NGINX_DIR=$(mktemp -d /tmp/bench-nginx.XXXXXX)
mkdir "$NGINX_DIR/logs"
# By the pid, not `nginx -s stop`: that parses the configuration again,
# and one that failed to load leaves the server it was meant to stop.
stop_nginx() {
  if [ -f "$NGINX_DIR/nginx.pid" ]; then
    master=$(cat "$NGINX_DIR/nginx.pid")
    kill -TERM "$master" 2>/dev/null || true
    while kill -0 "$master" 2>/dev/null; do sleep 0.2; done
  fi
  rm -rf "$NGINX_DIR"
}
trap stop_nginx EXIT
trap 'exit 1' INT TERM

openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 1 \
  -subj "/CN=$HOST" -addext "subjectAltName=DNS:$HOST" \
  -keyout "$NGINX_DIR/key.pem" -out "$NGINX_DIR/cert.pem" 2>/dev/null

cat >"$NGINX_DIR/nginx.conf" <<EOF
worker_processes 2;
worker_rlimit_nofile 65536;
pid $NGINX_DIR/nginx.pid;
error_log $NGINX_DIR/error.log warn;

events { worker_connections 4096; }

http {
    access_log off;
    keepalive_requests 1000000;

    upstream container {
        server $UPSTREAM;
        keepalive 256;
    }

    server {
        listen $NGINX_PORT ssl;
        http2 on;
        ssl_certificate     $NGINX_DIR/cert.pem;
        ssl_certificate_key $NGINX_DIR/key.pem;

        location / {
            proxy_pass http://container;
            proxy_http_version 1.1;
            proxy_set_header Connection "";
            proxy_set_header Host \$host;
            proxy_set_header X-Forwarded-Proto https;
            proxy_set_header X-Forwarded-Host \$host;
        }
    }
}
EOF
nginx -p "$NGINX_DIR" -c "$NGINX_DIR/nginx.conf"
NGINX_MASTER=$(cat "$NGINX_DIR/nginx.pid")
NGINX_PIDS="$NGINX_MASTER $(pgrep -P "$NGINX_MASTER" | tr '\n' ' ')"

url() {
  case $1 in
  direct) echo "http://$UPSTREAM" ;;
  nginx) echo "https://$HOST:$NGINX_PORT" ;;
  edge) echo "https://$HOST" ;;
  esac
}

pids() {
  case $1 in
  direct) ;;
  nginx) echo "$NGINX_PIDS" ;;
  edge) echo "$EDGE" ;;
  esac
}

ticks() {
  for pid in $(pids "$1"); do awk '{ print $14 + $15 }' "/proc/$pid/stat"; done |
    awk '{ n += $1 } END { print n + 0 }'
}

rss_mb() {
  for pid in $(pids "$1"); do awk '/^VmRSS/ { print $2 }' "/proc/$pid/status"; done |
    awk '{ n += $1 } END { printf "%.1f", n / 1024 }'
}

printf '%s\n' "edge pid $EDGE ($(/usr/local/bin/wabot-deploy --version)), nginx $(nginx -v 2>&1 | cut -d/ -f2) pids $NGINX_PIDS"
printf '%s\n' "$(nproc) cpus, ${SECS}s per run, $(date -u +%FT%TZ)"
printf '%s\n' "rss before: edge $(rss_mb edge) MB, nginx $(rss_mb nginx) MB"
echo
printf '%-20s %-6s %9s %9s %9s %9s %10s %12s %6s %8s\n' \
  "scenario" "via" "req/s" "p50" "p99" "max" "transfer" "proxy µs/req" "errors" "vs nginx"

# One line per run: `run <scenario> <via> <generator…>`, where the URL in
# the generator's arguments is already the one for <via>.
run() {
  scenario=$1 via=$2
  shift 2
  before=$(ticks "$via")
  out=$("$@" 2>&1) || true
  after=$(ticks "$via")

  case "$1" in
  h2load)
    reqs=$(printf '%s\n' "$out" | awk '/^requests:/ { print $6 }')
    rps=$(printf '%s\n' "$out" | awk '/^finished in/ { print $4 }')
    mbs=$(printf '%s\n' "$out" | awk '/^finished in/ { print $6 }')
    p50=$(printf '%s\n' "$out" | awk '$1 == "request" { print $5 }')
    p99=$(printf '%s\n' "$out" | awk '$1 == "request" { print $7 }')
    max=$(printf '%s\n' "$out" | awk '$1 == "request" { print $4 }')
    errs=$(printf '%s\n' "$out" | awk '/^requests:/ { print $10 + $12 + $14 }')
    ;;
  wrk)
    reqs=$(printf '%s\n' "$out" | awk '/requests in/ { print $1 }')
    rps=$(printf '%s\n' "$out" | awk '/^Requests\/sec/ { print $2 }')
    mbs=$(printf '%s\n' "$out" | awk '/^Transfer\/sec/ { print $2 "/s" }')
    p50="-" p99="-" max="-"
    errs=$(printf '%s\n' "$out" | awk '
      /Socket errors/ { gsub(",", ""); n += $4 + $6 + $8 + $10 }
      /Non-2xx/ { n += $NF }
      END { print n + 0 }')
    ;;
  esac

  per="-"
  if [ "${reqs:-0}" -gt 0 ] 2>/dev/null; then
    [ "$via" != direct ] && per=$(awk -v t="$((after - before))" -v hz="$HZ" -v n="$reqs" \
      'BEGIN { printf "%.0f", t * 1000000 / hz / n }')
  else
    printf '%s\n' "$out" >&2
  fi

  ratio=""
  case $via in
  nginx) nginx_rps=${rps:-} ;;
  edge) [ -n "${nginx_rps:-}" ] && [ -n "${rps:-}" ] &&
    ratio=$(awk -v e="$rps" -v n="$nginx_rps" 'BEGIN { printf "%.2fx", e / n }') ;;
  esac

  printf '%-20s %-6s %9s %9s %9s %9s %10s %12s %6s %8s\n' \
    "$scenario" "$via" "${rps:--}" "${p50:--}" "${p99:--}" "${max:--}" "${mbs:--}" "$per" "${errs:--}" "$ratio"
}

for via in direct nginx edge; do
  h2load --h1 -c4 -D2 "$(url "$via")/" >/dev/null 2>&1 || true
done

for c in 1 16 64 256; do
  nginx_rps=""
  for via in direct nginx edge; do
    run "h1 c=$c" "$via" h2load --h1 -m1 -D"$SECS" -c"$c" "$(url "$via")/"
  done
  echo
done

# HTTP/2 to the proxy and HTTP/1.1 from it: the two hops negotiate
# separately, and this is the path every browser takes. The container
# speaks only HTTP/1.1, so there is no direct line.
for shape in "1 1" "16 10"; do
  set -- $shape
  nginx_rps=""
  for via in nginx edge; do
    run "h2 c=$1 m=$2" "$via" h2load -m"$2" -D"$SECS" -c"$1" "$(url "$via")/"
  done
  echo
done

# A connection per request, which through a proxy is a TLS handshake each
# time: what a client that does not keep its connection costs.
nginx_rps=""
for via in direct nginx edge; do
  run "h1 close c=16" "$via" wrk -t1 -c16 -d"${SECS}s" -H "Connection: close" "$(url "$via")/"
done
echo

nginx_rps=""
for via in direct nginx edge; do
  run "h1 $BIG c=4" "$via" h2load --h1 -m1 -D"$SECS" -c4 "$(url "$via")$BIG"
done
echo

nginx_rps=""
for via in nginx edge; do
  run "h2 $BIG c=4" "$via" h2load -m1 -D"$SECS" -c4 "$(url "$via")$BIG"
done
echo

printf '%s\n' "rss after: edge $(rss_mb edge) MB, nginx $(rss_mb nginx) MB"
