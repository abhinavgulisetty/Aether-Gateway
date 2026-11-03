#!/usr/bin/env python3
"""Download pinned native Linux x86_64 tools into .local/tools; no sudo.
Requires Python, tar, and (for NGINX) a C compiler + make. --bench-only skips
Prometheus/Grafana/node_exporter (~600 MiB of additional archives).
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import urllib.request

root=Path(__file__).resolve().parents[1]
if platform.system()!='Linux' or platform.machine() not in ('x86_64','amd64'):
    raise SystemExit('Pinned binaries target Linux x86_64; install native tools for your platform instead.')
tools=Path(os.environ.get('AETHER_TOOLS_DIR',root/'.local/tools')).resolve()
tools.mkdir(parents=True,exist_ok=True)
versions=json.loads((root/'scripts/tool-versions.json').read_text())
for name,info in versions.items():
    if '--bench-only' in sys.argv and name not in ('oha','nginx'):continue
    archive=tools/info['archive']
    if not archive.exists():
        print('Downloading',name,info['version'],flush=True)
        partial=archive.with_suffix('.part')
        urllib.request.urlretrieve(info['url'],partial);partial.rename(archive)
    actual=hashlib.file_digest(open(archive,'rb'),'sha256').hexdigest()
    if actual!=info['sha256']:raise SystemExit(f'Checksum mismatch: {archive}; remove it and try again.')
    if name=='oha':archive.chmod(0o755);continue
    if (tools/name).exists():continue
    subprocess.run(['tar','-xzf',str(archive),'-C',str(tools)],check=True)
    if name=='nginx':
        source=tools/('nginx-'+info['version'])
        subprocess.run(['./configure','--prefix='+str(tools/'nginx'),'--without-http_rewrite_module','--without-http_gzip_module'],cwd=source,check=True)
        subprocess.run(['make','-j2'],cwd=source,check=True);subprocess.run(['make','install'],cwd=source,check=True)
    else:
        extracted=tools/(name+'-'+info['version']+('' if name=='grafana' else '.linux-amd64'))
        extracted.rename(tools/name)
print('Tools available in',tools)
