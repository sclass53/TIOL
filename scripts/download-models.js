const https = require('https');
const fs = require('fs');
const path = require('path');

const DEST = process.argv[2] || 'models-download';
const FILES = [
  { name: 'vision_model_int8.onnx', url: 'https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/vision_model_int8.onnx', size: 94553333 },
  { name: 'text_model_int8.onnx', url: 'https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/text_model_int8.onnx', size: 283438275 },
  { name: 'tokenizer.json', url: 'https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/tokenizer.json', size: 34363039 },
];

function download(file, destFile, tmpFile, retries, redirects) {
  return new Promise((resolve, reject) => {
    if (fs.existsSync(destFile) && fs.statSync(destFile).size === file.size) return resolve(true);
    let offset = fs.existsSync(tmpFile) ? fs.statSync(tmpFile).size : 0;
    if (offset > file.size) { try { fs.unlinkSync(tmpFile); } catch (e) {} offset = 0; }
    const headers = offset > 0 ? { Range: 'bytes=' + offset + '-' } : {};
    const req = https.get(file.url, { headers, timeout: 30000 }, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && redirects > 0) {
        res.resume();
        const loc = res.headers.location;
        file.url = loc ? new URL(loc, file.url).href : file.url;
        return download(file, destFile, tmpFile, retries, redirects - 1).then(resolve, reject);
      }
      if (res.statusCode === 416) {
        res.resume();
        try { fs.unlinkSync(tmpFile); } catch (e) {}
        return download(file, destFile, tmpFile, retries, redirects).then(resolve, reject);
      }
      if (res.statusCode !== 200 && res.statusCode !== 206) {
        res.resume();
        return reject(new Error('HTTP ' + res.statusCode));
      }
      const out = fs.createWriteStream(tmpFile, { flags: offset > 0 ? 'a' : 'w' });
      res.pipe(out);
      let last = Date.now();
      res.on('data', () => {
        if (Date.now() - last > 8000) { last = Date.now(); console.log(file.name + ': ' + (fs.statSync(tmpFile).size / 1048576).toFixed(1) + ' MB'); }
      });
      out.on('finish', () => {
        const sz = fs.statSync(tmpFile).size;
        if (sz !== file.size) {
          console.log(file.name + ': size mismatch ' + sz + ' != ' + file.size);
          return retries > 0 ? download(file, destFile, tmpFile, retries - 1, 5).then(resolve, reject) : reject(new Error('size mismatch'));
        }
        fs.renameSync(tmpFile, destFile);
        console.log(file.name + ': DONE (' + (sz / 1048576).toFixed(1) + ' MB)');
        resolve(true);
      });
      res.on('error', (e) => { out.destroy(); reject(e); });
    });
    req.on('error', (e) => {
      if (retries > 0) { console.log(file.name + ': error ' + e.message + ', retry ' + retries); setTimeout(() => download(file, destFile, tmpFile, retries - 1, 5).then(resolve, reject), 2000); }
      else reject(e);
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
  });
}

(async () => {
  fs.mkdirSync(DEST, { recursive: true });
  for (const f of FILES) {
    try { await download(f, path.join(DEST, f.name), path.join(DEST, f.name + '.part'), 8, 5); }
    catch (e) { console.error(f.name + ': FAILED ' + e.message); }
  }
  console.log('ALL DONE');
})();
