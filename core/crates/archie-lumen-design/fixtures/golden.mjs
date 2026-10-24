// Regenerates the golden fixtures of archie-lumen-design from Xavi's original source.
//
//   node core/crates/archie-lumen-design/fixtures/golden.mjs "<path to codigo fuente>"
//
// It runs the original modules themselves, unmodified, with node (v20.17 was used):
// field.js and visual-presets/catalog.js are imported as they are, and the settings
// literals at the top of main.js (SETTINGS, PROFILES, LEGACY_SETTINGS) are evaluated as
// written, since main.js itself needs a browser. Only the outputs are committed, never
// the original code.
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const source = process.argv[2];
if (!source) {
  console.error('usage: node golden.mjs <codigo fuente>');
  process.exit(2);
}
const here = path.dirname(fileURLToPath(import.meta.url));
const load = (file) => import(pathToFileURL(path.join(source, file)).href);
const write = (name, value, format = (v) => JSON.stringify(v, null, 1)) =>
  writeFileSync(path.join(here, name), `${format(value)}\n`);
// One row per line: a value whose members are all plain (or arrays of plain) is one line.
const plain = (v) => v === null || typeof v !== 'object' || (Array.isArray(v) && v.every((x) => x === null || typeof x !== 'object'));
const rows = (v, indent = '') => {
  if (plain(v) || (!Array.isArray(v) && Object.values(v).every(plain))) return JSON.stringify(v);
  const inner = `${indent} `;
  const items = Array.isArray(v) ? v.map((x) => inner + rows(x, inner))
    : Object.entries(v).map(([k, x]) => `${inner}${JSON.stringify(k)}: ${rows(x, inner)}`);
  return Array.isArray(v) ? `[\n${items.join(',\n')}\n${indent}]` : `{\n${items.join(',\n')}\n${indent}}`;
};
const origin = `codigo fuente, evaluated by fixtures/golden.mjs with node ${process.version}`;

// Presets: the settings literals of main.js, in the scope main.js gives them.
const catalog = await load('src/visual-presets/catalog.js');
const main = readFileSync(path.join(source, 'src/main.js'), 'utf8');
const start = main.indexOf('const SETTINGS = {');
const end = main.indexOf('const controls = {');
if (start < 0 || end < start) throw new Error('main.js no longer has SETTINGS before controls');
const literals = new Function('VISUAL_DEFAULTS', 'PRESETS',
  `${main.slice(start, end)}\nreturn { SETTINGS, PROFILES, ORIGINAL_PROFILE_OPTIONS, LEGACY_SETTINGS };`);
const { SETTINGS, PROFILES, ORIGINAL_PROFILE_OPTIONS, LEGACY_SETTINGS } =
  literals(catalog.VISUAL_DEFAULTS, catalog.PRESETS);
write('presets.golden.json', {
  origin: `${origin}: src/main.js SETTINGS, PROFILES, LEGACY_SETTINGS; src/visual-presets/catalog.js PRESETS`,
  defaults: SETTINGS,
  // The composition profiles of the panel; applying one is { ...SETTINGS, ...PROFILES[id] }.
  original: ORIGINAL_PROFILE_OPTIONS.map(([id, name]) => ({ id, name, overrides: PROFILES[id] })),
  // The "Antes" comparison look, whole.
  legacy: LEGACY_SETTINGS,
  presets: catalog.PRESETS.map(({ id, name, settings }) => ({ id, name, settings })),
});

// Layout: buildField of field.js over a fixed tree drawn from a fixed seed.
const { buildField } = await load('src/field.js');
function mulberry32(seed) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const seed = 20261023;
const random = mulberry32(seed);
const pick = (list) => list[Math.floor(random() * list.length)];
const extensions = ['rs', 'js', 'ts', 'py', 'cpp', 'h', 'qml', 'md', 'txt', 'pdf', 'png', 'jpg', 'mp4', 'svg',
  'json', 'toml', 'yaml', 'zip', 'gz', 'exe', 'so', 'sh', 'lock', 'bin', ''];
const stems = ['main', 'Lib', 'mod', 'README', 'index', 'Field', 'scene', 'data_set', 'a-b', 'notes', 'Zeta',
  'alpha', 'build', 'v2', 'Test10', 'test2', '_draft', 'x.y', 'Config', 'util'];
const entries = [{ id: '.', name: 'ArchieOS-Nvidia', parent: null, kind: 'dir', depth: 0, size: 0 }];
const add = (parent, name, kind, size) => {
  const id = parent === '.' ? name : `${parent}/${name}`;
  const depth = parent === '.' ? 1 : entries.find((e) => e.id === parent).depth + 1;
  entries.push({ id, name, parent, kind, depth, size });
  return id;
};
// Top level: folders of very different mass and many one-node files, so the region
// order depends on the subtree counts and, between equals, on localeCompare.
for (const name of ['README.md', 'AGENTS.md', 'Cargo.toml', 'build.rs', '.gitignore', 'LICENSE', 'Makefile',
  'readme.txt', 'Zeta.md', 'alpha.md', 'a-b.txt', 'a_b.txt', 'a.b.txt', 'file10.txt', 'file2.txt', 'File1.txt'])
  add('.', name, 'file', Math.floor(random() * 50000));
const grow = (parent, depth, budget) => {
  const folders = depth < 4 ? Math.floor(random() * 4) : 0;
  const files = 1 + Math.floor(random() * budget);
  for (let k = 0; k < files; k += 1) {
    const extension = pick(extensions);
    const name = `${pick(stems)}${k}${extension ? `.${extension}` : ''}`;
    if (!entries.some((e) => e.id === `${parent}/${name}`)) add(parent, name, 'file', Math.floor(random() ** 3 * 4e8));
  }
  for (let k = 0; k < folders; k += 1) grow(add(parent, `${pick(stems)}_${depth}${k}`, 'dir', 0), depth + 1, budget);
};
for (const [name, budget] of [['core', 14], ['display', 9], ['docs', 6], ['_dev', 3], ['Assets', 5], ['scripts', 2],
  ['tests', 4], ['node-tools', 2], ['vendor_libs', 1], ['3rdparty', 1], ['empty', 0]]) {
  const id = add('.', name, 'dir', 0);
  if (budget) grow(id, 2, budget);
}
const total = entries.length + 37;
const shape = (settings) => ({ layoutMode: settings.layoutMode, categoryPull: settings.categoryPull,
  hierarchyPull: settings.hierarchyPull, radialLayers: settings.radialLayers,
  sizeInfluence: settings.sizeInfluence, scale: settings.scale });
const cases = [
  ['field', { ...SETTINGS }, 0.72],
  ['field-no-cohesion', { ...SETTINGS }, 0],
  ...['crown', 'atlas', 'dense'].map((id) => [id, { ...SETTINGS, ...PROFILES[id] }, 0.72]),
  ['legacy', { ...LEGACY_SETTINGS }, 0.72],
  ['legacy-no-cohesion', { ...LEGACY_SETTINGS }, 0],
].map(([name, settings, folderCohesion]) => {
  const options = { ...shape(settings), total, folderCohesion };
  const field = buildField(entries.map((e) => ({ ...e })), options);
  return {
    name, options, radius: field.radius,
    nodes: field.nodes.map((n) => ({ id: n.id, category: n.category, subtree: n.subtreeCount, position: [n.x, n.y, n.z] })),
  };
});
// localeCompare decides the sibling order; a probe of names over the characters file
// names use, sorted by it, pins the collation the port has to reproduce.
const alphabet = 'aAbBnNoOzZ09_-. ~(+#áàâäÁñÑéèÉüçÇ';
const probe = Array.from({ length: 600 }, () =>
  Array.from({ length: 1 + Math.floor(random() * 5) }, () => alphabet[Math.floor(random() * alphabet.length)]).join(''));
write('field.golden.json', {
  origin: `${origin}: src/field.js buildField; tree from mulberry32(${seed})`,
  locale: Intl.Collator().resolvedOptions().locale,
  entries, total, cases,
  collation: [...new Set(probe)].sort((a, b) => a.localeCompare(b)),
}, rows);
console.log(`presets.golden.json, field.golden.json: ${entries.length} entries, ${cases.length} cases`);
