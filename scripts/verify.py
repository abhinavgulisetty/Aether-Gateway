#!/usr/bin/env python3
"""Targeted local demonstration; starts isolated processes, records observations.
No test framework or external Python packages. Run after a release build.
"""
import concurrent.futures as futures
import http.client
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
PORTS = [19101, 19102, 19103]
GATEWAY = 19300
processes = []
records = []

def record(name, **values):
    row = dict(check=name, **values)
    records.append(row)
    print(json.dumps(row), flush=True)

def call(port, path='/', method='GET', body=None, headers=None, source=None):
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=8, source_address=source)
    try:
        conn.request(method, path, body=body, headers=({'x-api-key':'verify-key'} if headers is None else headers))
        res = conn.getresponse()
        data = res.read()
        return res.status, dict(res.getheaders()), data
    finally:
        conn.close()

def control(port, **values):
    call(port, '/__control', 'POST', json.dumps(values))

def stats():
    return [json.loads(call(p, '/__stats')[2]) for p in PORTS]

def spawn(command, log):
    with open(log, 'wb') as output:
        proc = subprocess.Popen(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
    processes.append(proc)
    return proc

def wait_ready(port):
    for _ in range(100):
        try:
            if call(port, '/ready')[0] == 200:
                return
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(.05)
    raise RuntimeError(f'{port} not ready')

def main(directory):
    origins = [f'http://127.0.0.1:{p}' for p in PORTS]
    upstreams = [spawn(['node','scripts/upstream.mjs',str(p)], directory / f'upstream-{p}.log') for p in PORTS]
    config = {'listen':f'127.0.0.1:{GATEWAY}', 'auth':{'enabled':True,'api_keys':['verify-key']},
        'rate_limit':{'enabled':True,'max_tokens':100000,'refill_rate':100000},
        'limits':{'max_in_flight':8,'max_body_bytes':1048576},
        'health':{'interval_ms':100,'timeout_ms':80},
        'resilience':{'response_timeout_ms':150,'request_timeout_ms':1500,'max_attempts':2,'retry_backoff_ms':10,'failure_threshold':2,'cooldown_ms':500},
        'routes':[{'path_prefix':prefix,'strip_prefix':True,'policy':policy,'upstreams':targets}
            for prefix,policy,targets in [('/rr','round_robin',origins),('/least','least_active',origins),('/single','round_robin',origins[:1])]]}
    config_file = directory / 'verify.json'
    config_file.write_text(json.dumps(config))
    gateway = spawn(['target/release/aether-gateway',str(config_file)], directory / 'gateway.log')
    wait_ready(GATEWAY)
    assert call(GATEWAY,'/rr/',headers={})[0] == 401
    record('authentication', missing_key=401, valid_key=call(GATEWAY,'/rr/')[0])
    status, headers, data = call(GATEWAY,'/rr/echo?status=207&x=a%2Fb','PATCH',b'hello-stream',
        {'x-api-key':'verify-key','connection':'keep-alive, x-remove','x-remove':'secret','x-keep':'yes'})
    echo = json.loads(data)
    assert status == 207 and echo['method']=='PATCH' and echo['body']=='hello-stream'
    assert echo['path']=='/echo?status=207&x=a%2Fb' and echo['headers']['x-keep']=='yes'
    assert 'x-remove' not in echo['headers'] and 'x-hop-response' not in headers
    # Repeated Connection fields must all be sanitized.
    conn = http.client.HTTPConnection('127.0.0.1',GATEWAY)
    conn.putrequest('GET','/rr/echo'); conn.putheader('x-api-key','verify-key')
    conn.putheader('Connection','x-first'); conn.putheader('Connection','x-second')
    conn.putheader('x-first','remove'); conn.putheader('x-second','remove'); conn.endheaders()
    echo2 = json.loads(conn.getresponse().read()); conn.close()
    assert 'x-first' not in echo2['headers'] and 'x-second' not in echo2['headers']
    record('forwarding',method=echo['method'],status=status,path=echo['path'],body=echo['body'],multiple_connection_headers='stripped')
    before=stats()
    # One persistent downstream, shared upstream pool across all route requests.
    conn=http.client.HTTPConnection('127.0.0.1',GATEWAY)
    for _ in range(60):
        conn.request('GET','/rr/',headers={'x-api-key':'verify-key'}); assert conn.getresponse().read()
    conn.close()
    after=stats()
    distribution=[a['requests']-b['requests'] for a,b in zip(after,before)]
    accepted=[a['accepted']-b['accepted'] for a,b in zip(after,before)]
    assert distribution == [20,20,20] and sum(accepted) < 10
    record('round_robin_and_pooling',requests=60,distribution=distribution,new_upstream_sockets_including_stats=accepted)
    control(PORTS[0],delay_ms=80)
    def workload(prefix):
        before=stats(); started=time.monotonic()
        with futures.ThreadPoolExecutor(max_workers=6) as pool:
            results=list(pool.map(lambda _:call(GATEWAY,prefix+'/')[0],range(90)))
        assert set(results)=={200}
        return {'distribution':[a['requests']-b['requests'] for a,b in zip(stats(),before)],'seconds':round(time.monotonic()-started,3)}
    rr=workload('/rr'); least=workload('/least')
    assert least['distribution'][0] < rr['distribution'][0]
    record('uneven_latency',slow_backend_ms=80,round_robin=rr,least_active=least)
    control(PORTS[0],delay_ms=0)
    upstreams[1].terminate(); upstreams[1].wait(timeout=4); time.sleep(.35)
    responses=[call(GATEWAY,'/rr/') for _ in range(30)]
    assert all(s==200 and h['x-upstream']!=str(PORTS[1]) for s,h,_ in responses)
    record('upstream_failure',stopped=PORTS[1],successful_requests=30,traffic_to_stopped=0)
    upstreams[1]=spawn(['node','scripts/upstream.mjs',str(PORTS[1])],directory/'recovered.log'); time.sleep(.4)
    recovered=[call(GATEWAY,'/rr/')[1]['x-upstream'] for _ in range(12)]
    assert str(PORTS[1]) in recovered
    record('upstream_recovery',restarted=PORTS[1],distribution={p:recovered.count(str(p)) for p in PORTS})
    control(PORTS[0],status=503)
    before=stats()[0]['requests']; status=call(GATEWAY,'/single/echo','POST',b'once')[0]
    after=stats()[0]['requests']; assert status==503 and after-before==1
    record('non_idempotent_no_retry',status=status,upstream_attempts=after-before)
    # Two failed bodyless attempts open the circuit; single-upstream retries stop
    # when the circuit opens instead of bypassing its exclusion.
    call(GATEWAY,'/single/')
    circuit_text=call(GATEWAY,'/metrics')[2].decode()
    assert 'aether_circuit_state{route="/single",upstream="http://127.0.0.1:19101"} 1' in circuit_text
    assert call(GATEWAY,'/single/')[0]==503
    control(PORTS[0],status=200);time.sleep(.6)
    assert call(GATEWAY,'/single/')[0]==200
    record('circuit_breaker',open_state_observed=True,half_open_recovered=True)
    # A failure on one peer is retried against a healthy peer.
    control(PORTS[0],status=503)
    results=[call(GATEWAY,'/rr/')[0] for _ in range(12)]
    assert set(results)=={200}
    control(PORTS[0],status=200)
    record('bounded_idempotent_retry',requests=12,statuses=sorted(set(results)))
    control(PORTS[0],delay_ms=400);time.sleep(.6)
    started=time.monotonic();status=call(GATEWAY,'/single/')[0];elapsed=time.monotonic()-started
    assert status in (503,504) and elapsed<1
    record('response_timeout',status=status,seconds=round(elapsed,3),per_attempt_ms=150)
    control(PORTS[0],delay_ms=0);time.sleep(.6)
    started=time.monotonic();status,_,data=call(GATEWAY,'/rr/stream?chunks=512')
    assert status==200 and len(data)==32*1024*1024
    record('streaming',bytes=len(data),seconds=round(time.monotonic()-started,3))
    conn=http.client.HTTPConnection('127.0.0.1',GATEWAY,timeout=8)
    conn.request('GET','/rr/stream?chunks=256&interval_ms=20',headers={'x-api-key':'verify-key'})
    response=conn.getresponse();received=0;truncated=False
    try:
        while chunk:=response.read(65536):received+=len(chunk)
    except (http.client.IncompleteRead,OSError):truncated=True
    conn.close();assert truncated and received<16*1024*1024
    record('body_deadline',truncated=True,received_bytes=received,total_deadline_ms=1500)
    for port in PORTS: control(port,delay_ms=100)
    with futures.ThreadPoolExecutor(max_workers=24) as pool:
        statuses=list(pool.map(lambda _:call(GATEWAY,'/least/')[0],range(48)))
    assert 503 in statuses
    record('admission_backpressure',limit=8,statuses={s:statuses.count(s) for s in set(statuses)})
    for port in PORTS: control(port,delay_ms=0)
    time.sleep(.6)
    # Known oversized body rejected before forwarding.
    conn=http.client.HTTPConnection('127.0.0.1',GATEWAY)
    conn.request('POST','/rr/echo',headers={'x-api-key':'verify-key','Content-Length':str(1048576+1)})
    assert conn.getresponse().status==413;conn.close()
    record('body_limit',known_length_status=413)
    assert call(GATEWAY,'/rr/'+('x'*65520))[0]==414
    record('long_uri',forwarded_uri_overflow_status=414,gateway_still_alive=call(GATEWAY,'/health')[0]==200)
    # A separate small bucket demonstrates burst exhaustion without waiting for
    # the main scenario's large bucket to refill.
    config['listen']='127.0.0.1:19301';config['rate_limit']={'max_tokens':2,'refill_rate':.001,'max_clients':2,'idle_seconds':1}
    config_file=directory/'rate.json';config_file.write_text(json.dumps(config))
    spawn(['target/release/aether-gateway',str(config_file)],directory/'rate.log');wait_ready(19301)
    rates=[call(19301,'/rr/')[0] for _ in range(3)]
    assert rates==[200,200,429]
    assert call(19301,'/rr/',source=('127.0.0.2',0))[0]==200
    assert call(19301,'/rr/',source=('127.0.0.3',0))[0]==429
    time.sleep(1.1);assert call(19301,'/rr/',source=('127.0.0.3',0))[0]==200
    record('rate_limit',burst_statuses=rates,client_capacity_enforced=True,idle_expiry=True)
    text=call(GATEWAY,'/metrics')[2].decode();(directory/'metrics.prom').write_text(text)
    assert 'process_resident_memory_bytes' in text and 'aether_events_total' in text
    record('metrics',process_collector=True,retries_timeouts_health=True)
    conn=http.client.HTTPConnection('127.0.0.1',GATEWAY)
    conn.request('GET','/least/stream?chunks=256&interval_ms=20',headers={'x-api-key':'verify-key'})
    res=conn.getresponse();res.read(65536);res.close();conn.close();time.sleep(.2)
    cancellation_metrics=call(GATEWAY,'/metrics')[2].decode()
    cancellation_line=next(line for line in cancellation_metrics.splitlines() if line.startswith('aether_events_total{event="cancelled",route="/least"}'))
    assert float(cancellation_line.rsplit(' ',1)[1])>=1
    assert 'aether_active_requests{route="/least"} 0' in cancellation_metrics
    record('client_disconnect',cancellation_counted=True,active_requests_released=True)
    # SIGTERM during a finite in-flight stream must allow that stream to finish.
    conn=http.client.HTTPConnection('127.0.0.1',GATEWAY)
    conn.request('GET','/rr/stream?chunks=20&interval_ms=20',headers={'x-api-key':'verify-key'})
    res=conn.getresponse();gateway.send_signal(signal.SIGTERM);data=res.read();conn.close()
    gateway.wait(timeout=5);assert len(data)==20*65536 and gateway.returncode==0
    record('graceful_shutdown',in_flight_bytes=len(data),exit_code=gateway.returncode)

if __name__=='__main__':
    directory=Path(os.environ.get('VERIFY_OUTPUT', tempfile.mkdtemp(prefix='aether-verify-'))).resolve()
    directory.mkdir(parents=True,exist_ok=True)
    try:
        main(directory)
        (directory/'results.json').write_text(json.dumps(records,indent=2)+'\n')
        print(f'Evidence: {directory}')
    finally:
        for proc in reversed(processes):
            if proc.poll() is None:
                proc.terminate()
                try:proc.wait(timeout=5)
                except subprocess.TimeoutExpired:proc.kill();proc.wait()
