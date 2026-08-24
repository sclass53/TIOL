// Local HTTP proxy: forwards cargo sparse-index requests to the ustc mirror.
// Needed because schannel TLS is blocked in this environment while Node TLS works.
// Start: node scripts/cargo-proxy.js   (serves on 127.0.0.1:8013)
const http = require('http');
const https = require('https');

const PORT = 8013;
const INDEX_BASE = 'https://mirrors.ustc.edu.cn/crates.io-index/';
const CRATES_BASE = 'https://mirrors.ustc.edu.cn/crates.io/crates/';
const API_BASE = 'https://mirrors.ustc.edu.cn/crates.io/api/v1/crates/';

function fetch(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https:') ? https : http;
    mod.get(url, { headers: { 'User-Agent': 'cargo-local-proxy' } }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }));
    }).on('error', reject);
  });
}

http.createServer(async (req, res) => {
  const path = req.url;
  try {
    let upstream;
    if (path.startsWith('/index/')) {
      upstream = INDEX_BASE + path.slice('/index/'.length);
    } else if (path.startsWith('/crates/')) {
      // cargo asks {dl}/{name}/{version}/download; mirror layout is {name}/{name}-{version}.crate
      const m = path.match(/^\/crates\/([^\/]+)\/([^\/]+)\/download$/);
      if (m) upstream = CRATES_BASE + m[1] + '/' + m[1] + '-' + m[2] + '.crate';
      else upstream = CRATES_BASE + path.slice('/crates/'.length);
    } else if (path.startsWith('/api/')) {
      upstream = API_BASE + path.slice('/api/v1/crates/'.length);
    } else if (path === '/config.json' || path === '/index/config.json') {
      upstream = INDEX_BASE + 'config.json';
    } else {
      upstream = INDEX_BASE + path.replace(/^\//, '');
    }
    const r = await fetch(upstream);
    let body = r.body;
    if (path === '/config.json' || path === '/index/config.json') {
      // rewrite dl to this proxy so cargo downloads .crate files through us
      try {
        const cfg = JSON.parse(body.toString('utf8'));
        cfg.dl = `http://127.0.0.1:${PORT}/crates`;
        body = Buffer.from(JSON.stringify(cfg));
      } catch (e) { /* pass through */ }
    }
    res.writeHead(r.status, { 'content-type': r.headers['content-type'] || 'application/json' });
    res.end(body);
  } catch (e) {
    res.writeHead(502);
    res.end('proxy error: ' + e.message);
  }
}).listen(PORT, '127.0.0.1', () => console.log('cargo proxy on 127.0.0.1:' + PORT));


