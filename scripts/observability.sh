#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
tools_dir="${AETHER_TOOLS_DIR:-$PWD/.local/tools}"
state_dir="$PWD/.local/observability"
mkdir -p "$state_dir"
stop() {
  for name in prometheus grafana node_exporter; do
    if [[ -f "$state_dir/$name.pid" ]]; then kill -TERM "$(cat "$state_dir/$name.pid")" 2>/dev/null || true; rm -f "$state_dir/$name.pid"; fi
  done
}
if [[ "${1:-start}" == stop ]]; then stop; exit; fi
start() {
  local name="$1"; shift
  if [[ -f "$state_dir/$name.pid" ]] && kill -0 "$(cat "$state_dir/$name.pid")" 2>/dev/null; then echo "$name already running"; return; fi
  setsid "$@" < /dev/null > "$state_dir/$name.log" 2>&1 & echo "$!" > "$state_dir/$name.pid"
}
start prometheus "$tools_dir/prometheus/prometheus" --config.file=observability/prometheus.yml --web.listen-address=127.0.0.1:9090 --storage.tsdb.path="$state_dir/prometheus-data" --storage.tsdb.retention.time=2d
start node_exporter "$tools_dir/node_exporter/node_exporter" --web.listen-address=127.0.0.1:9200
start grafana "$tools_dir/grafana/bin/grafana" server --homepath="$tools_dir/grafana" --config="$PWD/observability/grafana.ini" cfg:default.paths.data="$state_dir/grafana-data" cfg:default.paths.logs="$state_dir" cfg:default.paths.provisioning="$PWD/observability/grafana/provisioning"
echo 'Prometheus http://127.0.0.1:9090  Grafana http://127.0.0.1:3005/d/aether-systems'
