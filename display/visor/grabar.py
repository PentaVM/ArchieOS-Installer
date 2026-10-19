#!/usr/bin/env python3
"""Records ONCE what the Lumenfield core computes over the OS content folder: every API answer the
sphere uses (graph for each profile and budget, thinking links, details and children of every
node). Output: /srv/visor/snap/*.json, the saved graph that servidor.py serves. Needs the core
running headless over that folder (see grabar.sh)."""
import json, os, sys, time, urllib.request, urllib.parse, hashlib

URL = open(os.environ.get('LUMEN_URLFILE', '/srv/lf/url.txt')).read().strip()
BASE = URL.split('/?')[0]
TOKEN = URL.split('token=')[1]
OUT = '/srv/visor/snap'
os.makedirs(OUT, exist_ok=True)

def call(path, body=None, tries=1):
    req = urllib.request.Request(BASE + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={'X-Archie-Token': TOKEN, 'Content-Type': 'application/json'},
                                 method='POST' if body is not None else 'GET')
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())

def key(method, path, body=None):
    raw = method + ' ' + path + (json.dumps(body, sort_keys=True) if body is not None else '')
    return hashlib.sha1(raw.encode()).hexdigest()[:16]

index = {}
def save(method, path, data, body=None):
    k = key(method, path, body)
    json.dump(data, open(f'{OUT}/{k}.json', 'w'), separators=(',', ':'), ensure_ascii=False)
    index[method + ' ' + path + ('' if body is None else ' ' + json.dumps(body, sort_keys=True))] = k

def rec(path, body=None):
    try:
        d = call(path, body)
    except Exception as e:
        print('FALLO', path, e, file=sys.stderr); return None
    save('GET' if body is None else 'POST', path, d, body)
    return d

# wait until the core has read the folder
for _ in range(600):
    try:
        s = call('/api/state')
        if s.get('loaded'): break
    except Exception: pass
    time.sleep(2)
print('estado', s)

for p in ['/api/state', '/api/prefs', '/api/session', '/api/profiles', '/api/processes', '/api/overview?focus=.',
          '/api/watch', '/api/searches', '/api/index/saved', '/api/update/channel']:
    rec(p)

# thinking links (notes): computed once and kept
job = rec('/api/notes/start', {'focus': '.', 'force': False})
for _ in range(300):
    st = call('/api/notes/status')
    if st.get('status') in ('complete', 'failed', 'error', 'cancelled'): break
    time.sleep(2)
save('GET', '/api/notes/status', st)
print('notas', st.get('status'))
rec('/api/notes/relations?limit=60000')

BUDGETS = [30, 60, 120, 220, 400, 800, 1500, 3000, 6000, 12000, 24000, 36000]
full = None
for cap in (0, 10, 14):
    for b in BUDGETS:
        q = '/api/graph?focus=.&target=&limit=36000' + (f'&cap={cap}' if cap else '') + f'&budget={b}'
        d = rec(q)
        if cap == 0 and b == 36000: full = d
# no budget (non-sphere profiles)
rec('/api/graph?focus=.&target=&limit=36000')

nodes = full['nodes'] if full else []
print('nodos en el grafo completo:', len(nodes))
n = 0
for node in nodes:
    nid = node['id']
    q = urllib.parse.quote(nid, safe='')
    rec('/api/details?id=' + q)
    if node.get('kind') == 'dir':
        rec(f'/api/children?folder={q}&limit=300')
        rec(f'/api/children?folder={q}&deep=1&limit=500')
    else:
        rec('/api/notes/note?id=' + q)
    n += 1
json.dump(index, open(f'{OUT}/_indice.json', 'w'), ensure_ascii=False)
print('GRABADO', len(index), 'respuestas')
