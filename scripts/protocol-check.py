#!/usr/bin/env python3
"""Exercise cleartext downstream h2 and verified HTTPS upstream ALPN.
The HTTPS check calls Cloudflare's public trace endpoint; stores no client IP.
"""
import http.client,json,os,subprocess,tempfile,time
from pathlib import Path
root=Path(__file__).resolve().parents[1]
output=Path(os.environ.get('PROTOCOL_OUTPUT',tempfile.mkdtemp(prefix='aether-protocol-'))).resolve();output.mkdir(parents=True,exist_ok=True)
config={'listen':'127.0.0.1:19311','rate_limit':{'enabled':False},'health':{'path':'/','interval_ms':1000,'timeout_ms':5000},
    'resilience':{'connect_timeout_ms':3000,'response_timeout_ms':5000,'request_timeout_ms':10000},
    'routes':[{'path_prefix':'/local','strip_prefix':True,'upstream':'http://127.0.0.1:19111'},
              {'path_prefix':'/remote','strip_prefix':True,'upstream':'https://www.cloudflare.com'}]}
(output/'config.json').write_text(json.dumps(config,indent=2))
processes=[]
def launch(command,name):
    with open(output/(name+'.log'),'wb') as log:processes.append(subprocess.Popen(command,cwd=root,stdout=log,stderr=subprocess.STDOUT))
def call(path):
    c=http.client.HTTPConnection('127.0.0.1',19311,timeout=15);c.request('GET',path);r=c.getresponse();data=r.read();c.close();return r.status,data
try:
    launch(['node','scripts/upstream.mjs','19111'],'upstream')
    launch(['target/release/aether-gateway',str(output/'config.json')],'gateway')
    for _ in range(100):
        try:
            if call('/local/')[0]==200:break
        except OSError:pass
        time.sleep(.1)
    oha=os.environ.get('OHA',str(root/'.local/tools/oha'))
    raw=subprocess.check_output([oha,'--http2','-n','100','-c','4','--no-tui','--output-format','json','http://127.0.0.1:19311/local/'])
    (output/'downstream-h2.json').write_bytes(raw);h2=json.loads(raw)
    assert h2['statusCodeDistribution']=={'200':100} and not h2['errorDistribution']
    remote=None
    for _ in range(5):
        status,body=call('/remote/cdn-cgi/trace')
        if status==200:
            fields=dict(line.split('=',1) for line in body.decode().splitlines() if '=' in line)
            remote={'status':status,**{key:fields.get(key) for key in ['http','tls','visit_scheme']}}
            break
        time.sleep(1)
    result={'downstream_cleartext_http2':{'successful_requests':100,'errors':0},'verified_https_upstream':remote or {'status':status,'available':False}}
    if remote:assert remote['http'] in ('h2','http/2') and remote['visit_scheme']=='https'
    (output/'results.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
finally:
    for p in reversed(processes):
        p.terminate()
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:p.kill();p.wait()
