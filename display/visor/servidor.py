#!/usr/bin/env python3
"""ArchieOS graph viewer: serves the Archie Lumenfield 0449 sphere (its web engine, in web/) over
the SAVED graph (/srv/visor/snap, recorded once by grabar.py). It never runs the Rust core and
never scans at start-up. A watcher adds the files that appear later in the OS content folder
(/srv/archieos) to that graph, keeps them in /srv/visor/nuevos.json, and the page refreshes itself.
Arch/Linux only."""
import json, os, re, sys, threading, time, urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get('VISOR_PORT', '8765'))
ROOT = os.environ.get('VISOR_ROOT', '/srv/archieos')
SNAP = '/srv/visor/snap'
WEB = os.environ.get('VISOR_WEB', os.path.join(os.path.dirname(os.path.abspath(__file__)), 'web'))
NUEVOS = '/srv/visor/nuevos.json'
URLFILE = os.environ.get('VISOR_URLFILE', '/srv/visor/url.txt')

INDEX = json.load(open(f'{SNAP}/_indice.json', encoding='utf-8'))
_cache = {}
def snap(k):
    if k not in _cache:
        _cache[k] = json.load(open(f'{SNAP}/{INDEX[k]}.json', encoding='utf-8'))
    return _cache[k]

# ---- new files -------------------------------------------------------------------------------
lock = threading.Lock()
nuevos = {}          # id -> node
version = [0]
if os.path.exists(NUEVOS):
    try: nuevos = json.load(open(NUEVOS, encoding='utf-8'))
    except Exception: nuevos = {}

def conocidos():
    """Every id in the saved (complete) graph."""
    full = snap('GET /api/graph?focus=.&target=&limit=36000&budget=36000')
    return {n['id'] for n in full['nodes']}
def recorrer():
    out = {}
    for d, dirs, files in os.walk(ROOT):
        dirs[:] = [x for x in dirs if x != '.git']
        rel = os.path.relpath(d, ROOT).replace(os.sep, '/')
        for name in dirs + files:
            rid = name if rel == '.' else f'{rel}/{name}'
            out[rid] = (rel if rel != '.' else '.', name, os.path.join(d, name))
    return out
BASEFILE = '/srv/visor/base-ids.json'
if not os.path.exists(BASEFILE):   # what the folder held when the graph was recorded
    json.dump(sorted(recorrer()), open(BASEFILE, 'w', encoding='utf-8'))
BASE_IDS = conocidos() | set(json.load(open(BASEFILE, encoding='utf-8')))

def vigilar():
    while True:
        try:
            visto = recorrer()
            cambio = False
            with lock:
                for rid, (parent, name, path) in visto.items():
                    if rid in BASE_IDS or rid in nuevos: continue
                    try: st = os.stat(path)
                    except OSError: continue
                    isdir = os.path.isdir(path)
                    nuevos[rid] = {'id': rid, 'name': name, 'parent': parent, 'kind': 'dir' if isdir else 'file',
                                   'size': 4096 if isdir else st.st_size, 'modified': int(st.st_mtime * 1000),
                                   'depth': rid.count('/') + 1, 'nuevo': True}
                    cambio = True
                for rid in [r for r in nuevos if r not in visto]:
                    del nuevos[rid]; cambio = True
                if cambio:
                    version[0] += 1
                    json.dump(nuevos, open(NUEVOS, 'w', encoding='utf-8'), ensure_ascii=False)
        except Exception as e:
            print('vigilante:', e, file=sys.stderr)
        time.sleep(3)

def con_nuevos(g):
    """The saved graph plus the new files (each linked to its folder when that folder is in the graph)."""
    with lock:
        extra = list(nuevos.values())
    if not extra: return g
    g = dict(g)
    ids = {n['id'] for n in g['nodes']}
    nodes = list(g['nodes']); links = list(g['links'])
    for n in extra:
        if n['id'] in ids or n['parent'] not in ids: continue
        nodes.append(n); links.append({'source': n['parent'], 'target': n['id']}); ids.add(n['id'])
    g['nodes'] = nodes; g['links'] = links; g['total'] = g.get('total', 0) + len(extra)
    return g

# ---- recorded API ----------------------------------------------------------------------------
def graph_for(path):
    q = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
    cap = int((q.get('cap') or ['0'])[0] or 0); budget = int((q.get('budget') or ['0'])[0] or 0)
    if budget <= 0:
        return snap('GET /api/graph?focus=.&target=&limit=36000')
    pool = sorted({int(re.search(r'budget=(\d+)$', k).group(1)) for k in INDEX
                   if k.startswith('GET /api/graph?') and 'budget=' in k and ((f'&cap={cap}&' in k) if cap else ('&cap=' not in k))})
    if not pool: return snap('GET /api/graph?focus=.&target=&limit=36000')
    pick = next((b for b in pool if b >= budget), pool[-1])
    k = f'GET /api/graph?focus=.&target=&limit=36000' + (f'&cap={cap}' if cap else '') + f'&budget={pick}'
    return snap(k)

def full_nodes():
    full = snap('GET /api/graph?focus=.&target=&limit=36000&budget=36000')
    t = {n['id']: n for n in full['nodes']}
    with lock: t.update(nuevos)
    return t

def respond(method, path, body):
    p = urllib.parse.urlsplit(path)
    route = p.path
    if route == '/api/graph':
        return 200, con_nuevos(graph_for(path))
    if route == '/api/nodes' and method == 'POST':
        t = full_nodes(); ids = (json.loads(body or '{}').get('ids') or [])
        return 200, {'nodes': [t[i] for i in ids if i in t]}
    if route == '/api/scan/start':
        return 200, {'elapsedMs': 0, 'errors': 0, 'found': len(full_nodes()), 'id': 2, 'root': ROOT, 'status': 'complete', 'truncated': False}
    if route == '/api/scan/status':
        return 200, {'elapsedMs': 0, 'errors': 0, 'found': len(full_nodes()), 'id': 2, 'root': ROOT, 'status': 'complete', 'truncated': False}
    if route == '/api/state':
        s = dict(snap('GET /api/state')); s['total'] = len(full_nodes()); return 200, s
    if route == '/api/details':
        k = 'GET ' + path
        if k in INDEX: return 200, snap(k)
        nid = (urllib.parse.parse_qs(p.query).get('id') or [''])[0]
        n = full_nodes().get(nid)
        if n: return 200, {'errors': 0, 'node': n, 'path': os.path.join(ROOT, nid), 'root': ROOT, 'summary': None, 'truncated': False}
        return 404, {'error': 'sin datos'}
    if route == '/api/children':
        k = 'GET ' + path
        if k in INDEX:
            d = snap(k)
            return 200, d
        return 200, {'nodes': []}
    k = f'{method} {path}' + ('' if method == 'GET' or not body else ' ' + json.dumps(json.loads(body), sort_keys=True))
    if k in INDEX: return 200, snap(k)
    k2 = 'GET ' + path
    if k2 in INDEX: return 200, snap(k2)
    if route.startswith('/api/notes/start') or route.startswith('/api/notes/status'):
        return 200, snap('GET /api/notes/status')
    if route == '/api/notes/relations' and 'GET /api/notes/relations?limit=60000' in INDEX:
        return 200, snap('GET /api/notes/relations?limit=60000')
    return 404, {'error': 'no disponible en el visor'}

# ---- page: only the sphere -------------------------------------------------------------------
INYECTADO = r"""
<style id="visor-css">
#rail,.rail-tabs,.topbar,.minimap,#minimap,.dock-group,.brand,.sphere-legend,.toast,#toasts{display:none!important}
.workspace{position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;margin:0!important}
#inspector{position:fixed!important;right:16px!important;top:46px!important;width:340px!important;max-height:68vh!important;overflow:auto!important;z-index:30;background:rgba(2,6,10,.92)!important;border:1px solid rgba(30,192,255,.4)!important;border-radius:12px!important}
#inspector>*:not(#selected-section){display:none!important}#inspector:has(.selection-empty){display:none!important}
</style>
<script>
(function(){
  var v=-1;
  setInterval(function(){fetch('/visor/version').then(function(r){return r.json()}).then(function(j){
    if(v>=0&&j.v!==v){var b=document.getElementById('refresh-button');if(b)b.click();}
    v=j.v;}).catch(function(){});},3000);
})();
</script>
"""
TIPOS = {'.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json',
         '.png': 'image/png', '.jpg': 'image/jpeg', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.ttf': 'font/ttf',
         '.webp': 'image/webp', '.ktx2': 'application/octet-stream', '.glb': 'model/gltf-binary', '.wasm': 'application/wasm'}

class H(BaseHTTPRequestHandler):
    def log_message(self, fmt, *a):
        if os.environ.get('VISOR_LOG'): print(self.address_string(), fmt % a, flush=True)
    def _send(self, code, data, tipo='application/json'):
        raw = data if isinstance(data, bytes) else json.dumps(data, separators=(',', ':'), ensure_ascii=False).encode()
        if code >= 400: print('RESP', code, self.command, self.path, flush=True)
        self.send_response(code); self.send_header('Content-Type', tipo); self.send_header('Content-Length', str(len(raw)))
        self.send_header('Cache-Control', 'no-store'); self.end_headers(); self.wfile.write(raw)
    def _api(self, method):
        n = int(self.headers.get('Content-Length') or 0)
        body = self.rfile.read(n).decode() if n else ''
        if method == 'PUT':
            # preferences and profiles: accepted, nothing is stored
            p = urllib.parse.urlsplit(self.path).path
            k = 'GET ' + p
            return self._send(200, snap(k) if k in INDEX else (json.loads(body) if body else {}))
        code, data = respond(method, self.path, body)
        self._send(code, data)
    def do_GET(self):
        p = urllib.parse.urlsplit(self.path).path
        if p == '/visor/version': return self._send(200, {'v': version[0]})
        if p.startswith('/api/'): return self._api('GET')
        if p == '/': p = '/index.html'
        f = os.path.normpath(os.path.join(WEB, p.lstrip('/')))
        if not f.startswith(WEB) or not os.path.isfile(f): return self._send(404, b'no encontrado', 'text/plain')
        data = open(f, 'rb').read()
        if p == '/index.html': data = data.replace(b'</body>', INYECTADO.encode() + b'</body>', 1)
        self._send(200, data, TIPOS.get(os.path.splitext(f)[1].lower(), 'application/octet-stream'))
    def do_POST(self): self._api('POST')
    def do_PUT(self): self._api('PUT')

if __name__ == '__main__':
    threading.Thread(target=vigilar, daemon=True).start()
    srv = ThreadingHTTPServer(('127.0.0.1', PORT), H)
    open(URLFILE, 'w').write(f'http://127.0.0.1:{PORT}/?token=visor')
    print('visor en', PORT, flush=True)
    srv.serve_forever()
