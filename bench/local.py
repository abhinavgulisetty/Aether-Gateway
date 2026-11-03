#!/usr/bin/env python3
"""Native closed-loop comparative workload; no containers or test framework.
Requires node, oha, nginx, Rust. All results include raw oha JSON + /proc samples.
"""
import concurrent.futures
import hashlib
import http.client
import json
import os
from pathlib import Path
import platform
import resource
import shutil
import statistics
import subprocess
import threading
import time

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT / os.environ.get('RESULT_DIR',time.strftime('bench/results/local-%Y%m%dT%H%M%SZ',time.gmtime()))
OUT.mkdir(parents=True,exist_ok=True)
TOOLS=Path(os.environ.get('AETHER_TOOLS_DIR', ROOT/'.local/tools'))
OHA=os.environ.get('OHA',str(TOOLS/'oha'))
NGINX=os.environ.get('NGINX',str(TOOLS/'nginx/sbin/nginx'))
DURATION=os.environ.get('DURATION','5s')
WARMUP=os.environ.get('WARMUP','2s')
RUNS=int(os.environ.get('RUNS','3'))
CONCURRENCIES=[int(x) for x in os.environ.get('CONCURRENCIES','1,32,128').split(',')]
PROCS=[]

def run(command, **kwargs):
    return subprocess.run(command,check=True,cwd=ROOT,**kwargs)

def start(command,name,cwd=ROOT):
    output=open(os.devnull if name.startswith('original') else OUT/f'{name}.log','wb')
    process=subprocess.Popen(command,cwd=cwd,stdout=output,stderr=subprocess.STDOUT)
    output.close(); PROCS.append(process); return process

def wait(port,path='/health'):
    for _ in range(100):
        try:
            c=http.client.HTTPConnection('127.0.0.1',port,timeout=.5);c.request('GET',path)
            r=c.getresponse();r.read();c.close()
            if r.status==200:return
        except OSError:pass
        time.sleep(.05)
    raise RuntimeError(f'Failed to start {port}')

def pids(pid):
    found=[pid]
    try:
        children=Path(f'/proc/{pid}/task/{pid}/children').read_text().split()
        for child in children:found.extend(pids(int(child)))
    except FileNotFoundError:pass
    return found

def proc_stats(pid):
    ticks=0;rss=0
    for p in pids(pid):
        try:
            parts=Path(f'/proc/{p}/stat').read_text().split(') ',1)[1].split()
            ticks+=int(parts[11])+int(parts[12]);rss+=int(parts[21])*os.sysconf('SC_PAGE_SIZE')
        except FileNotFoundError:pass
    return ticks/os.sysconf('SC_CLK_TCK'),rss

def main():
    # Preserve original compilation flags; release profiles differ and are
    # explicitly part of the comparison rather than a controlled single change.
    run(['cargo','build','--release','--locked'])
    run(['cargo','build','--release','--locked','--manifest-path','bench/baseline/Cargo.toml','--target-dir',str(ROOT/'.local/baseline-target')])
    original_source=ROOT/'.local/original-source';original_source.mkdir(parents=True,exist_ok=True)
    archive=run(['git','archive','446d66b'],stdout=subprocess.PIPE).stdout
    subprocess.run(['tar','-x','-C',str(original_source)],input=archive,check=True)
    # git archive restores old mtimes; invalidate Cargo's mtime fingerprint after
    # restoring source previously compiled with a different limiter constant.
    for restored in original_source.rglob('*.rs'): os.utime(restored,None)
    run(['cargo','build','--release','--locked','--manifest-path',str(original_source/'Cargo.toml'),'--target-dir',str(ROOT/'.local/original-target')])
    original_binary=OUT/'aether-original'
    shutil.copy2(ROOT/'.local/original-target/release/aether-gateway',original_binary)
    # The original limiter ignores YAML and always enforces 10 burst / 2 rps.
    # Keep it intact in `original`; one explicit constant-only variant enables
    # a data-path comparison. Its exact diff is saved alongside the results.
    limiter_path=original_source/'src/middleware/rate_limit.rs'
    text=limiter_path.read_text();assert 'TokenBucket::new(10.0, 2.0)' in text
    limiter_path.write_text(text.replace('TokenBucket::new(10.0, 2.0)','TokenBucket::new(1e12, 1e12)'))
    (OUT/'original-unlimited.patch').write_text('-TokenBucket::new(10.0, 2.0)\n+TokenBucket::new(1e12, 1e12)\n')
    run(['cargo','build','--release','--locked','--manifest-path',str(original_source/'Cargo.toml'),'--target-dir',str(ROOT/'.local/original-target')])
    binaries={'original-unlimited':ROOT/'.local/original-target/release/aether-gateway','original':original_binary,
              'previous':Path(os.environ.get('PREVIOUS_BINARY',ROOT/'.local/baseline-target/release/aether-gateway')),
              'enhanced':ROOT/'target/release/aether-gateway'}
    assert hashlib.sha256(binaries['original'].read_bytes()).digest()!=hashlib.sha256(binaries['original-unlimited'].read_bytes()).digest(), 'baseline artifacts must differ'
    upstream=start(['node','scripts/upstream.mjs','19201'],'upstream');wait(19201)
    processes={};targets={'direct':19201,'original':19202,'previous':19203,'enhanced':19204,'nginx':19205,'original-unlimited':19206}
    for name in binaries:
        config={'listen':f'127.0.0.1:{targets[name]}','auth':{'enabled':False},'rate_limit':{'enabled':False},
                'routes':[{'path_prefix':'/','upstream':'http://127.0.0.1:19201'}]}
        config_path=OUT/f'{name}.json';config_path.write_text(json.dumps(config,indent=2))
        runtime=OUT/(name+'-runtime');runtime.mkdir(exist_ok=True)
        (runtime/'config.yaml').write_text(json.dumps(config))
        processes[name]=start([str(binaries[name].resolve()),str(config_path.resolve())],name,cwd=runtime.resolve())
        wait(targets[name],'/ready' if name=='enhanced' else '/')
    # Prove original rate limiting before recording a comparison.
    original_statuses=[]
    for _ in range(20):
        c=http.client.HTTPConnection('127.0.0.1',19202);c.request('GET','/');res=c.getresponse();res.read();original_statuses.append(res.status);c.close()
    assert 429 in original_statuses, 'original must retain its hardcoded limiter'
    (OUT/'baseline-validation.json').write_text(json.dumps({'original_statuses':original_statuses,'distinct_binary_hashes':True},indent=2))
    nginx_config=OUT/'nginx.conf'
    nginx_config.write_text(f'''worker_processes auto;
pid {OUT.resolve()}/nginx.pid;
error_log {OUT.resolve()}/nginx-error.log warn;
events {{ worker_connections 4096; }}
http {{
  access_log off;
  upstream local_backend {{ server 127.0.0.1:19201; keepalive 32; }}
  server {{ listen 127.0.0.1:19205;
    location / {{ proxy_http_version 1.1; proxy_set_header Connection ""; proxy_pass http://local_backend; }}
  }}
}}
''')
    processes['nginx']=start([NGINX,'-p',str(OUT.resolve())+'/', '-c',str(nginx_config.resolve()),'-g','daemon off;'],'nginx');wait(19205,'/')
    processes['direct']=upstream
    diagnostic=http.client.HTTPConnection('127.0.0.1',19201)
    def backend_stats():
        diagnostic.request('GET','/__stats');return json.loads(diagnostic.getresponse().read())
    backend_stats()
    environment={'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'platform':platform.platform(),
        'cpu':run(['lscpu'],stdout=subprocess.PIPE,text=True).stdout,'cargo':run(['cargo','--version'],stdout=subprocess.PIPE,text=True).stdout.strip(),
        'node':run(['node','--version'],stdout=subprocess.PIPE,text=True).stdout.strip(),'oha':run([OHA,'--version'],stdout=subprocess.PIPE,text=True).stdout.strip(),
        'nginx':subprocess.run([NGINX,'-v'],capture_output=True,text=True).stderr.strip(),
        'head':run(['git','rev-parse','HEAD'],stdout=subprocess.PIPE,text=True).stdout.strip(),
        'source_files_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [ROOT/'Cargo.toml',ROOT/'Cargo.lock',*sorted((ROOT/'src').rglob('*.rs'))]},
        'tracked_source_diff_sha256':hashlib.sha256(run(['git','diff','HEAD','--','src','Cargo.toml','Cargo.lock'],stdout=subprocess.PIPE).stdout).hexdigest(),
        'binary_sha256':{name:hashlib.sha256(path.read_bytes()).hexdigest() for name,path in binaries.items()},
        'duration':DURATION,'warmup':WARMUP,'runs':RUNS,'concurrencies':CONCURRENCIES,'loadavg_before':os.getloadavg(),
        'original_note':'original always enforces a hardcoded 10 burst / 2 rps even when YAML disables policy; original-unlimited changes only those two bucket constants to 1e12; rejected RPS is not proxy throughput',
        'method':'HTTP/1.1, keep-alive, 256-byte response; policy disabled for all proxies; enhanced telemetry/health/breaker remain enabled; sequential alternating targets; shared host'}
    (OUT/'environment.json').write_text(json.dumps(environment,indent=2))
    summaries=[]
    for concurrency in CONCURRENCIES:
        names=list(targets)
        for repeat in range(1,RUNS+1):
            for name in names if repeat%2 else list(reversed(names)):
                port=targets[name];url=f'http://127.0.0.1:{port}/'
                common=[OHA,'--no-tui','--output-format','json','-w','-t','10s','-c',str(concurrency)]
                run(common+['-z',WARMUP,url],stdout=subprocess.DEVNULL)
                before=backend_stats();cpu_before,_=proc_stats(processes[name].pid);up_cpu_before,_=proc_stats(upstream.pid)
                samples=[];done=threading.Event()
                def monitor():
                    while not done.is_set():
                        cpu,rss=proc_stats(processes[name].pid)
                        samples.append({'elapsed':time.monotonic()-started,'cpu_seconds':cpu,'rss_bytes':rss})
                        done.wait(.1)
                children_before=resource.getrusage(resource.RUSAGE_CHILDREN)
                started=time.monotonic();thread=threading.Thread(target=monitor);thread.start()
                path=OUT/f'{name}-c{concurrency}-r{repeat}.json'
                try:
                    with open(path,'wb') as output:run(common+['-z',DURATION,url],stdout=output)
                finally:done.set();thread.join()
                elapsed=time.monotonic()-started
                children_after=resource.getrusage(resource.RUSAGE_CHILDREN)
                cpu_after,rss=proc_stats(processes[name].pid);up_cpu_after,_=proc_stats(upstream.pid);after=backend_stats()
                raw=json.loads(path.read_text());summary=raw['summary'];latency=raw['latencyPercentiles']
                record={'target':name,'concurrency':concurrency,'run':repeat,'rps':summary['requestsPerSec'],'successful_rps':sum(count for code,count in raw.get('statusCodeDistribution',{}).items() if 200 <= int(code) < 300)/summary['total'],
                    'p50_ms':latency['p50']*1000,'p95_ms':latency['p95']*1000,'p99_ms':latency['p99']*1000,
                    'cpu_percent':100*(cpu_after-cpu_before)/elapsed,'generator_cpu_percent':100*((children_after.ru_utime+children_after.ru_stime)-(children_before.ru_utime+children_before.ru_stime))/elapsed,'upstream_cpu_percent':100*(up_cpu_after-up_cpu_before)/elapsed,
                    'peak_rss_mib':max([rss]+[s['rss_bytes'] for s in samples])/1048576,
                    'upstream_new_connections':after['accepted']-before['accepted'],
                    'upstream_requests':after['requests']-before['requests'],
                    'statuses':raw.get('statusCodeDistribution',{}),'errors':raw.get('errorDistribution',{}),'elapsed_seconds':elapsed}
                (OUT/f'{name}-c{concurrency}-r{repeat}-resources.json').write_text(json.dumps({'summary':record,'samples':samples},indent=2))
                summaries.append(record);print(json.dumps(record),flush=True)
                (OUT/'measurements.json').write_text(json.dumps(summaries,indent=2))
    diagnostic.close()
    lines=['| Target | Concurrency | All-response RPS | 2xx RPS | p50 ms | p95 ms | p99 ms | CPU % | Peak RSS MiB |','|---|---:|---:|---:|---:|---:|---:|---:|---:|']
    for c in CONCURRENCIES:
        for name in targets:
            records=[r for r in summaries if r['target']==name and r['concurrency']==c]
            med=lambda k:statistics.median(r[k] for r in records)
            lines.append(f'| {name} | {c} | {med("rps"):,.0f} | {med("successful_rps"):,.0f} | {med("p50_ms"):.3f} | {med("p95_ms"):.3f} | {med("p99_ms"):.3f} | {med("cpu_percent"):.1f} | {med("peak_rss_mib"):.1f} |')
    (OUT/'summary.md').write_text('\n'.join(lines)+'\n')
    print('\n'.join(lines))

try:
    main()
finally:
    for process in reversed(PROCS):
        if process.poll() is None:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
    (OUT/'aether-original').unlink(missing_ok=True)
