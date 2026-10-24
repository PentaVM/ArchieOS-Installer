export const text = value => typeof value === 'string' ? value : '';
export const list = value => Array.isArray(value) ? value : [];
const fold = value => text(value).normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLocaleLowerCase('es');
const code = value => fold(value).replace(/[-\s]/g, '_');
export const escapeHTML = value => text(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

export function standing(value, basis) {
  const state = code(value), origin = code(basis);
  if (['hypothesis','inferred','tentative','model_inferred','advisory'].includes(state) || ['inferred','inferred_from_messages','model_inferred','co_presence_inference','hypothesis'].includes(origin))
    return {key:'inferred', label:'Interpretación'};
  if (!state && origin==='public_record') return {key:'asserted',label:'Registro público'};
  if (['asserted','confirmed','declared','verified','observed','corroborated'].includes(state) || (!state && ['owner_declared','subject_declared','participant_declared','declared','owner_asserted','subject_asserted'].includes(origin)))
    return {key:'asserted', label:'Declarada'};
  return {key:'unknown', label:'Sin confirmar'};
}

function traits(value) {
  return list(value).filter(t => t && typeof t === 'object').map(t => ({
    key:text(t.key), value:t.redacted ? '' : text(t.value), basis:text(t.basis),
    sensitivity:text(t.sensitivity), source_note:text(t.source_note), redacted:Boolean(t.redacted)
  }));
}

export function normalizeView(input) {
  const value = input && typeof input === 'object' ? input : {};
  const graph = value.graph && typeof value.graph === 'object' ? value.graph : {};
  const ids = new Set();
  const nodes = list(graph.nodes).filter(n => {
    if (!n || typeof n !== 'object' || !text(n.id) || ids.has(n.id)) return false;
    ids.add(n.id); return true;
  }).map(n => ({...n, id:text(n.id), label:text(n.label) || 'Personas sin nombre',
    kind:text(n.kind), degree:Number.isInteger(n.degree) && n.degree >= 0 ? n.degree : null,
    path_to_owner:list(n.path_to_owner).filter(p=>typeof p==='string'),
    tags:traits(n.tags), traits:traits(n.traits)}));
  return {
    schema_version:value.schema_version, domain:text(value.domain), generated_at_ms:value.generated_at_ms,
    availability:text(value.availability) || 'unconfigured',
    graph:{...graph,owner_id:text(graph.owner_id),nodes,edges:list(graph.edges).filter(e => e && ids.has(e.from_id) && ids.has(e.to_id)).map(e=>({...e}))},
    recommendations:list(value.recommendations).filter(r=>r && typeof r === 'object'),
    sources:list(value.sources).filter(s=>s && typeof s === 'object'),
    pending_events:Number.isFinite(value.pending_events) ? Math.max(0,value.pending_events) : 0
  };
}

export function filterNodes(nodes, query = '', tag = '') {
  const needle = fold(query.trim()), chosen = fold(tag);
  return nodes.filter(n => (!chosen || n.tags.some(t=>!t.redacted && fold(t.value)===chosen)) &&
    (!needle || fold([n.label,...n.tags.filter(t=>!t.redacted).map(t=>t.value),...n.traits.filter(t=>!t.redacted).map(t=>`${t.key} ${t.value}`)].join(' ')).includes(needle)));
}
export const selectedNode = (nodes, id) => nodes.find(n=>n.id===id) || null;
export const validView = (view, domain) => view?.schema_version===1 && (view.graph==null || view.graph.schema_version===1) && view?.domain===domain;

export function configuredDomains(input) {
  const seen = new Set();
  return list(Array.isArray(input) ? input : input?.domains).map(item => typeof item==='string' ? {domain:item,label:item} : {domain:text(item?.domain),label:text(item?.label) || text(item?.domain)})
    .filter(item => item.domain && !seen.has(item.domain) && seen.add(item.domain));
}

export function recommendationHTML(item, now = Date.now()) {
  const status = standing(item.standing), expired = Number.isFinite(item.expires_at_ms) && item.expires_at_ms > 0 && item.expires_at_ms <= now;
  const refs = list(item.evidence_refs).filter(r=>typeof r==='string');
  const alternatives = list(item.alternatives).filter(a=>typeof a==='string');
  return `<article class="recommendation ${expired?'expired':''}"><div class="card-meta"><span class="status ${status.key}">${expired?'Vencida':escapeHTML(status.label)}</span><span>${escapeHTML(text(item.kind).replaceAll('_',' '))}</span></div>
    <h3>${escapeHTML(item.title || 'Propuesta para revisar')}</h3><p>${escapeHTML(item.explanation)}</p>
    ${!expired && item.suggestion ? `<div class="suggestion">${escapeHTML(item.suggestion)}</div>` : ''}
    ${expired ? '<p class="muted">Actualiza las fuentes antes de considerar esta propuesta.</p>' : ''}
    ${alternatives.length ? `<details><summary>Otras interpretaciones</summary><ul>${alternatives.map(a=>`<li>${escapeHTML(a)}</li>`).join('')}</ul></details>` : ''}
    <details><summary>${refs.length ? `${refs.length} ${refs.length===1?'evidencia':'evidencias'}` : 'Sin evidencia citada'}</summary>${refs.length ? `<ul class="evidence">${refs.map(r=>`<li><code>${escapeHTML(r)}</code></li>`).join('')}</ul>` : '<p>No hay referencias disponibles.</p>'}</details>
    ${Number.isFinite(item.expires_at_ms) && item.expires_at_ms > 0 ? `<small>Revisar antes de ${escapeHTML(new Date(item.expires_at_ms).toLocaleString('es-CL'))}</small>` : ''}</article>`;
}

// A tidy relationship map. Asserted paths determine columns; hypotheses cannot
// give someone a degree. Each column has fixed, non-overlapping row spacing.
export function layoutGraph(graph) {
  const columns = new Map(), nodes = list(graph.nodes), byId = new Map(nodes.map(n=>[n.id,n]));
  const owner = byId.get(graph.owner_id);
  const adjacent = new Map(nodes.map(n=>[n.id,[]]));
  for (const edge of list(graph.edges)) {
    if (standing(edge.standing,edge.basis).key !== 'asserted' || !adjacent.has(edge.from_id) || !adjacent.has(edge.to_id)) continue;
    adjacent.get(edge.from_id).push(edge.to_id); adjacent.get(edge.to_id).push(edge.from_id);
  }
  const depths = new Map(owner ? [[owner.id,0]] : []), queue = owner ? [owner.id] : [];
  for (let i=0;i<queue.length;i++) for (const id of adjacent.get(queue[i])) {
    if (!depths.has(id)) {depths.set(id,depths.get(queue[i])+1); queue.push(id);}
  }
  const first = (adjacent.get(owner?.id) || []).sort((a,b)=>byId.get(a).label.localeCompare(byId.get(b).label,'es'));
  const side = new Map(first.map((id,i)=>[id,i%2===0?-1:1]));
  const branch = owner ? [owner.id] : [];
  for (let i=0;i<branch.length;i++) for (const id of adjacent.get(branch[i])) {
    if (id!==owner.id && !side.has(id) && depths.get(id)>depths.get(branch[i])) {side.set(id,side.get(branch[i]) || 1); branch.push(id);}
    else if (id!==owner.id && depths.get(id)===1 && !branch.includes(id)) branch.push(id);
  }
  const maxDepth = Math.max(1,...depths.values());
  for (const node of nodes) {
    const depth = depths.get(node.id);
    const col = depth===0 ? 0 : depth===undefined ? maxDepth+1 : depth*(side.get(node.id) || 1);
    if (!columns.has(col)) columns.set(col,[]);
    columns.get(col).push(node);
  }
  const output=[];
  for (const [col,group] of columns) {
    group.sort((a,b)=>a.label.localeCompare(b.label,'es'));
    group.forEach((node,i)=>output.push({...node,x:col*290,y:(i-(group.length-1)/2)*124,unresolved:!depths.has(node.id)}));
  }
  return output;
}
