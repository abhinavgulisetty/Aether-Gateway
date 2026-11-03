// Local diagnostic upstream. No npm packages. Its counters distinguish HTTP
// requests from accepted TCP sockets to demonstrate real keep-alive reuse.
import http from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
const port = Number(process.argv[2] || 9101);
let state = { delay_ms: 0, status: 200, fail_every: 0, health: true };
let accepted = 0, requests = 0, active = 0, peak = 0;
const payload = Buffer.alloc(256, 'a');
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname === '/health') { res.writeHead(state.health ? 200 : 503); res.end(); return; }
  if (url.pathname === '/__stats') {
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify({port,accepted,requests,active,peak,...state})); return;
  }
  if (url.pathname === '/__control' && req.method === 'POST') {
    let text = ''; for await (const b of req) { text += b; if (text.length > 4096) { res.writeHead(413); res.end(); return; } }
    try { Object.assign(state, JSON.parse(text)); res.end(JSON.stringify(state)); }
    catch { res.writeHead(400); res.end(); } return;
  }
  ++requests; ++active; peak = Math.max(peak, active);
  res.on('close', () => --active);
  const number = requests;
  try {
    const delayMs = Number(url.searchParams.get('delay_ms') ?? state.delay_ms);
    if (delayMs > 0) await delay(delayMs);
    if (res.destroyed) return;
    const status = Number(url.searchParams.get('status') ?? (state.fail_every && number % state.fail_every === 0 ? 503 : state.status));
    res.setHeader('x-upstream', String(port));
    if (url.pathname === '/echo') {
      let size = 0; const chunks = [];
      for await (const chunk of req) { size += chunk.length; if (size > 1048576) { res.writeHead(413); res.end(); return; } chunks.push(chunk); }
      res.writeHead(status, {'content-type':'application/json', 'connection':'keep-alive, x-hop-response', 'x-hop-response':'must-be-stripped'});
      res.end(JSON.stringify({method:req.method,path:req.url,headers:req.headers,body:Buffer.concat(chunks).toString()})); return;
    }
    if (url.pathname === '/stream') {
      const count = Math.min(4096, Number(url.searchParams.get('chunks') || 256));
      const chunk = Buffer.alloc(65536, 's');
      res.writeHead(status, {'content-type':'application/octet-stream'});
      for (let i=0; i<count && !res.destroyed; ++i) {
        if (!res.write(chunk)) await new Promise(resolve => {
          const finish = () => { res.off('drain', finish); res.off('close', finish); resolve(); };
          res.once('drain', finish); res.once('close', finish);
        });
        if (url.searchParams.has('interval_ms')) await delay(Number(url.searchParams.get('interval_ms')));
      }
      res.end(); return;
    }
    res.writeHead(status, {'content-type':'text/plain','content-length':payload.length});
    res.end(payload);
  } catch { res.destroy(); }
});
server.on('connection', socket => { ++accepted; socket.setNoDelay(true); });
server.keepAliveTimeout = 60000;
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({event:'listening',port})));
process.on('SIGTERM', () => { server.close(); server.closeIdleConnections(); setTimeout(()=>process.exit(0),2000).unref(); });
