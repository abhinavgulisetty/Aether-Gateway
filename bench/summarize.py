#!/usr/bin/env python3
"""Summarize current native results or the preserved historical oha files."""
import json
import statistics
import sys
from pathlib import Path

folder=Path(sys.argv[1])
measurements=folder/'measurements.json'
if measurements.exists():
    rows=json.loads(measurements.read_text())
    print('| Target | Concurrency | 2xx RPS | p50 ms | p95 ms | p99 ms | CPU % | Peak RSS MiB |')
    print('|---|---:|---:|---:|---:|---:|---:|---:|')
    for concurrency in sorted({r['concurrency'] for r in rows}):
        for name in dict.fromkeys(r['target'] for r in rows):
            group=[r for r in rows if r['concurrency']==concurrency and r['target']==name]
            if not group:continue
            med=lambda k:statistics.median(r[k] for r in group)
            print(f'| {name} | {concurrency} | {med("successful_rps"):,.0f} | {med("p50_ms"):.3f} | {med("p95_ms"):.3f} | {med("p99_ms"):.3f} | {med("cpu_percent"):.1f} | {med("peak_rss_mib"):.1f} |')
else:
    print('| Historical target | All-response RPS | p50 ms | p95 ms | p99 ms | Non-2xx |')
    print('|---|---:|---:|---:|---:|---:|')
    for name in ('direct','nginx','aether-bare','aether-full'):
        raw=[json.loads(p.read_text()) for p in folder.glob(name+'-*.json')]
        if not raw:continue
        fields=[(d['summary']['requestsPerSec'],d['latencyPercentiles']['p50']*1000,
            d['latencyPercentiles']['p95']*1000,d['latencyPercentiles']['p99']*1000,
            sum(n for s,n in d['statusCodeDistribution'].items() if not s.startswith('2'))) for d in raw]
        m=[statistics.median(values) for values in zip(*fields)]
        print(f'| {name} | {m[0]:,.0f} | {m[1]:.3f} | {m[2]:.3f} | {m[3]:.3f} | {m[4]:.0f} |')
