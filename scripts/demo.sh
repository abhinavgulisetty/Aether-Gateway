#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
state_dir="${AETHER_STATE_DIR:-$PWD/.local/run}"
mkdir -p "$state_dir"
start_process() {
  local name="$1"; shift
  if [[ -f "$state_dir/$name.pid" ]] && kill -0 "$(cat "$state_dir/$name.pid")" 2>/dev/null; then
    echo "$name already running"; return
  fi
  setsid "$@" < /dev/null > "$state_dir/$name.log" 2>&1 &
  echo "$!" > "$state_dir/$name.pid"
}
stop_process() {
  local name="$1"
  if [[ -f "$state_dir/$name.pid" ]]; then
    kill -TERM "$(cat "$state_dir/$name.pid")" 2>/dev/null || true
    rm -f "$state_dir/$name.pid"
  fi
}
case "${1:-help}" in
  start)
    cargo build --release --locked
    for port in 9101 9102 9103; do start_process "upstream-$port" node scripts/upstream.mjs "$port"; done
    start_process gateway target/release/aether-gateway "${CONFIG:-config.yaml}"
    echo "Gateway: http://127.0.0.1:3000  Metrics: /metrics  API key: local-demo-key"
    ;;
  stop)
    stop_process gateway
    for port in 9101 9102 9103; do stop_process "upstream-$port"; done
    ;;
  kill-upstream) stop_process "upstream-${2:-9102}" ;;
  recover-upstream) start_process "upstream-${2:-9102}" node scripts/upstream.mjs "${2:-9102}" ;;
  slow)
    curl -fsS -X POST "http://127.0.0.1:${2:-9102}/__control" -d '{"delay_ms":500}' ;;
  reset)
    for port in 9101 9102 9103; do curl -fsS -X POST "http://127.0.0.1:$port/__control" -d '{"delay_ms":0,"status":200,"fail_every":0,"health":true}'; done ;;
  traffic)
    oha -z "${DURATION:-20s}" -c "${CONNECTIONS:-32}" --no-tui -H 'x-api-key: local-demo-key' http://127.0.0.1:3000/api/ ;;
  *) echo "Usage: $0 {start|stop|kill-upstream [port]|recover-upstream [port]|slow [port]|reset|traffic}" ;;
esac
