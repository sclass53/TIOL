// Vendors all Cargo.lock crates into the local cargo cache with FIXED mtimes
// (Windows cannot restore the epoch-0 mtimes in some .crate tars, which
// breaks cargo's unpack step). Cargo does not re-verify cached .crate hashes.
const http = require('http');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const ROOT = path.resolve(__dirname, '..'); // E:\ImageManager
const CARGO_HOME = path.join(ROOT, '.cargo-home');
const CACHE_DIR = path.join(CARGO_HOME, 'registry', 'cache', '127.0.0.1-0c55ffa3db4746e5');
const TMP = path.join(CARGO_HOME, 'repack-tmp');

function download(url, tries) {
  tries = tries || 4;
  return new Promise((resolve, reject) => {
    http.get(url, { timeout: 180000 }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400) {
        res.resume();
        return download(res.headers.location, tries).then(resolve, reject);
      }
      if (res.statusCode !== 200) { res.resume(); return reject(new Error('HTTP ' + res.statusCode)); }
      const ch = [];
      res.on('data', (c) => ch.push(c));
      res.on('end', () => resolve(Buffer.concat(ch)));
    }).on('error', (e) => {
      if (tries > 1) { setTimeout(() => download(url, tries - 1).then(resolve, reject), 1500); }
      else reject(e);
    });
  });
}

function repack(name, version, buf) {
  const dir = path.join(TMP, name + '-' + version);
  fs.mkdirSync(dir, { recursive: true });
  const crateFile = path.join(TMP, name + '-' + version + '.crate');
  fs.writeFileSync(crateFile, buf);
  try { execFileSync('tar', ['-xzf', crateFile, '-C', dir], { stdio: 'ignore' }); } catch (e) { /* mtime restore warnings are fine */ }
  const out = path.join(CACHE_DIR, name + '-' + version + '.crate');
  execFileSync('tar', ['-czf', out, '--mtime=2020-01-01 00:00:00', '-C', dir, name + '-' + version], { stdio: 'ignore' });
  fs.rmSync(dir, { recursive: true, force: true });
  fs.rmSync(crateFile, { force: true });
  return out;
}

async function main() {
  fs.mkdirSync(CACHE_DIR, { recursive: true });
  fs.mkdirSync(TMP, { recursive: true });
  const lock = fs.readFileSync(path.join(ROOT, 'src-tauri', 'Cargo.lock'), 'utf8');
  const pkgs = [];
  const blocks = lock.split(/^\[\[package\]\]$/m).slice(1);
  for (const b of blocks) {
    const name = /^name = "([^"]+)"/m.exec(b);
    const ver = /^version = "([^"]+)"/m.exec(b);
    const checksum = /^checksum = "([^"]+)"/m.exec(b);
    if (name && ver && checksum) pkgs.push({ name: name[1], version: ver[1], checksum: checksum[1] });
  }
  console.log('total packages:', pkgs.length);
  let done = 0, skipped = 0, failed = 0;
  const concurrency = 6;
  let idx = 0;
  async function worker() {
    while (idx < pkgs.length) {
      const i = idx++;
      const p = pkgs[i];
      const cacheFile = path.join(CACHE_DIR, p.name + '-' + p.version + '.crate');
      if (fs.existsSync(cacheFile)) { skipped++; continue; }
      try {
        const buf = await download(`http://127.0.0.1:8013/crates/${p.name}/${p.version}/download`);
        repack(p.name, p.version, buf);
        done++;
        if (done % 40 === 0) console.log('repacked', done);
      } catch (e) {
        failed++;
        console.error('FAILED', p.name, p.version, e.message);
      }
    }
  }
  await Promise.all(Array.from({ length: concurrency }, worker));
  console.log(`done=${done} skipped=${skipped} failed=${failed}`);
}
main();


