import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeView, filterNodes, selectedNode, standing, escapeHTML, recommendationHTML, layoutGraph, configuredDomains, validView } from '../model.mjs';

const fixture = {
  schema_version: 1, domain: 'personal/owner', availability: 'ready',
  graph: { schema_version: 1, owner_id: 'owner', nodes: [
    { id: 'owner', label: 'Yo', kind: 'owner', degree: 0, tags: [], traits: [] },
    { id: 'ana', label: 'Ana Álvarez', kind: 'consented_person', degree: 1,
      tags: [{key:'wanda.tag',value:'Familia',basis:'owner_declared',redacted:false}],
      traits: [{key:'flor',value:'Tulipanes',basis:'subject_declared',redacted:false}] },
    { id: 'leo', label: 'Leo', degree: null,
      tags: [{key:'wanda.tag',value:'Amigos',basis:'inferred',redacted:false}, {key:'wanda.tag',value:'Oculto',redacted:true}], traits: null }
  ], edges: [{from_id:'owner',to_id:'ana',kind:'family',standing:'asserted'}, {from_id:'ana',to_id:'leo',kind:'friend',standing:'hypothesis'}] },
  recommendations: [], sources: [], pending_events: 0
};

// Catches accent-sensitive search and filters that ignore object-valued tags.
test('search normalizes accents and combines configured tag filters', () => {
  const view = normalizeView(fixture);
  assert.deepEqual(filterNodes(view.graph.nodes, 'alvarez', 'Familia').map(n=>n.id), ['ana']);
  assert.deepEqual(filterNodes(view.graph.nodes, 'ana', 'Amigos').map(n=>n.id), []);
  assert.deepEqual(filterNodes(view.graph.nodes, 'tulipanes', '').map(n=>n.id), ['ana']);
});
test('redacted tags never enter searchable labels', () => {
  assert.deepEqual(filterNodes(normalizeView(fixture).graph.nodes, 'amigos', '').map(n=>n.id), ['leo']);
  assert.deepEqual(filterNodes(normalizeView(fixture).graph.nodes, 'oculto', ''), []);
});
test('a selection outside the current domain or filter is cleared', () => {
  const nodes = normalizeView(fixture).graph.nodes;
  assert.equal(selectedNode(nodes, 'ana')?.label, 'Ana Álvarez');
  assert.equal(selectedNode(filterNodes(nodes, '', 'Amigos'), 'ana'), null);
  assert.equal(selectedNode(nodes, '__proto__'), null);
});
test('standing does not promote inferred or unknown relations', () => {
  assert.equal(standing('hypothesis', 'owner_declared').key, 'inferred');
  assert.equal(standing('asserted', 'owner_declared').key, 'asserted');
  assert.equal(standing(null, 'inferred').key, 'inferred');
  assert.equal(standing(null, 'owner_declared').key, 'asserted');
  assert.equal(standing('unexpected', 'unexpected').key, 'unknown');
});
test('C++ trait bases retain inference and public-record provenance', () => {
  assert.deepEqual(standing(null, 'inferred_from_messages'), {key:'inferred',label:'Interpretación'});
  assert.deepEqual(standing(null, 'public_record'), {key:'asserted',label:'Registro público'});
});
test('malformed optional fields render an empty view without selecting fabricated people', () => {
  const view = normalizeView({graph:{nodes:[null,{id:'',label:'bad'}],edges:null},recommendations:null,sources:null});
  assert.deepEqual(view.graph.nodes, []);
  assert.deepEqual(view.recommendations, []);
  assert.equal(view.availability, 'unconfigured');
  assert.equal(normalizeView(null).domain, '');
});
test('recommendation content and references cannot create HTML or event handlers', () => {
  const html = recommendationHTML({id:'x',title:'<img src=x onerror=alert(1)>',explanation:'<script>evil()</script>',
    suggestion:'" onclick="bad',standing:'inferred',evidence_refs:['<svg/onload=bad>'],alternatives:['<b>maybe</b>'],expires_at_ms:null}, 100);
  assert.ok(!html.includes('<img'));
  assert.ok(!html.includes('<script'));
  assert.ok(!html.includes('<svg'));
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('&lt;b&gt;maybe&lt;/b&gt;'));
  assert.equal(escapeHTML('"&<>\''), '&quot;&amp;&lt;&gt;&#39;');
});
test('expired recommendations are labelled and their suggested action is withheld', () => {
  const html = recommendationHTML({title:'Regalo',suggestion:'Comprar a 22.900',expires_at_ms:99,evidence_refs:[]},100);
  assert.ok(html.includes('Vencida'));
  assert.ok(!html.includes('Comprar a 22.900'));
});
test('graph layout gives unique positions and keeps unresolved people visible', () => {
  const positions = layoutGraph(normalizeView(fixture).graph);
  assert.equal(positions.length, 3);
  assert.equal(new Set(positions.map(p=>`${p.x},${p.y}`)).size, 3);
  assert.ok(positions.find(p=>p.id==='leo'));
  assert.deepEqual(layoutGraph({nodes:[],edges:[],owner_id:''}), []);
});
test('only server-provided domains become options', () => {
  assert.deepEqual(configuredDomains({domains:['personal/owner',{domain:'work/studio/app',label:'Estudio'},null,'']}),
    [{domain:'personal/owner',label:'personal/owner'},{domain:'work/studio/app',label:'Estudio'}]);
  assert.deepEqual(configuredDomains(null), []);
});
test('unconfigured null graph is accepted while cross-domain and future versions are refused', () => {
  assert.equal(validView({schema_version:1,domain:'personal/owner',graph:null},'personal/owner'),true);
  assert.equal(validView(fixture,'work/studio/app'),false);
  assert.equal(validView({...fixture,schema_version:2},'personal/owner'),false);
  assert.equal(validView({...fixture,graph:{schema_version:2}},'personal/owner'),false);
});
