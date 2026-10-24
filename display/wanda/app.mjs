import { text, list, normalizeView, filterNodes, selectedNode, standing, escapeHTML as esc, recommendationHTML, layoutGraph, configuredDomains, validView } from './model.mjs';

const $ = id => document.getElementById(id);
const STORE = 'wanda.pairedToken';
let token = '';
try { token = sessionStorage.getItem(STORE) || ''; } catch { /* In-memory pairing still works. */ }
let view = normalizeView(null), demo = false, selected = null, tag = '', mode = 'map', domains = [], generation = 0;
let pending = null, querying = null, busy = false, points = [], scale = 1, offset = {x:0,y:0}, dragging = null;
const kindLabels = {parent:'Familia · madre o padre',child:'Familia · hija o hijo',sibling:'Familia · hermanos',family:'Familia',friend:'Amistad',partner:'Pareja',manager:'Jefatura',colleague:'Compañeros de trabajo',employer:'Relación laboral',employee:'Relación laboral',household:'Hogar',acquaintance:'Conocidos',authorized_guest:'Visita autorizada'};
const labelFor = kind => kindLabels[text(kind).replaceAll('-','_')] || text(kind).replaceAll('_',' ').replaceAll('-',' ') || 'Relación sin especificar';
const initials = label => text(label).split(/\s+/).slice(0,2).map(s=>[...s][0] || '').join('').toUpperCase();
const names = () => new Map(view.graph.nodes.map(n=>[n.id,n.label]));
const visible = () => filterNodes(view.graph.nodes,$('search').value,tag);
function message(value, error = false) { $('notice').textContent = value; $('notice').hidden = !value; $('notice').classList.toggle('error',error); }
function date(value) { return Number.isFinite(value) && value>0 ? new Date(value).toLocaleString('es-CL',{dateStyle:'short',timeStyle:'short'}) : 'Tiempo no disponible'; }
function auth(value) { token=value; try {value ? sessionStorage.setItem(STORE,value) : sessionStorage.removeItem(STORE);} catch { /* No persistent fallback. */ } }

async function api(path, options = {}) {
  if (!token) throw new Error('pair');
  const response = await fetch(path,{...options,headers:{'Authorization':`Bearer ${token}`,...(options.body ? {'Content-Type':'application/json'} : {})},credentials:'omit',cache:'no-store',referrerPolicy:'no-referrer'});
  if (response.status===401 || response.status===403) throw new Error('auth');
  if (!response.ok) throw new Error(response.status===404 ? 'unsupported' : 'server');
  const payload = await response.text();
  if (payload.length>8*1024*1024) throw new Error('large');
  return JSON.parse(payload);
}
const errorCopy = err => ({pair:'Conecta Wanda con tu token local para ver este contexto.',auth:'El token no tiene acceso a este contexto. Revisa el emparejamiento local.',unsupported:'Esta instalación todavía no ofrece esta función de Wanda.',large:'La respuesta excede el tamaño permitido para la vista.'}[err.message] || 'No se pudo consultar Wanda. Comprueba que el servicio local esté disponible e intenta otra vez.');

function setDomains(options, active='') {
  domains=options;
  $('domain').replaceChildren(...(options.length?options:[{domain:'',label:'Sin configurar'}]).map(d=>{const option=document.createElement('option');option.value=d.domain;option.textContent=d.label;return option;}));
  $('domain').disabled=!options.length || demo;
  if (options.some(d=>d.domain===active)) $('domain').value=active;
}
async function connect() {
  const mine=++generation; pending?.abort(); querying?.abort(); pending=new AbortController();
  demo=false;view=normalizeView(null);selected=null;tag='';busy=false;render();
  $('connection').textContent='Conectando…';
  let configured;
  try { configured=configuredDomains(await api('/v1/domains',{signal:pending.signal})); }
  catch (err) {
    if (err.name==='AbortError' || mine!==generation) return;
    // The contract's configured default. Never synthesize work/personal alternatives.
    if (err.message==='unsupported') configured=[{domain:'personal/owner',label:'personal/owner'}];
    else {setDomains([]);$('connection').textContent='Sin conectar';message(errorCopy(err),true);throw err;}
  }
  if(mine!==generation)return;
  setDomains(configured);
  if(!configured.length){$('connection').textContent='Sin configurar';message('No hay dominios configurados para este token. Configura un contexto en el servicio local.');return;}
  await load();
}
async function load() {
  const domain=$('domain').value;
  if(!domain || demo)return;
  const mine=++generation;pending?.abort();querying?.abort();pending=new AbortController();busy=false;
  selected=null;tag='';view=normalizeView(null);render();message('Cargando el contexto…');$('connection').textContent='Conectando…';
  try {
    const raw=await api(`/v1/view?domain=${encodeURIComponent(domain)}`,{signal:pending.signal});
    if(mine!==generation)return;
    if(!validView(raw,domain)) throw new Error('schema');
    view=normalizeView(raw);selected=view.graph.owner_id || null;
    $('connection').textContent='Conectado · local';$('pair-button').textContent='Conexión';
    message(view.availability==='ready' || view.availability==='available' ? '' : 'El contexto está disponible de forma parcial. Revisa las fuentes y la configuración del servicio.');
    render();fit();
  } catch(err) {
    if(err.name==='AbortError' || mine!==generation)return;
    $('connection').textContent='Sin conectar';message(errorCopy(err),true);render();
  }
}

function renderTags() {
  const all = new Map();
  for (const node of view.graph.nodes) for (const t of node.tags) if(!t.redacted && t.value) {
    if (!all.has(t.value)) all.set(t.value,new Set());all.get(t.value).add(node.id);
  }
  $('tags').innerHTML = `<button class="tag-filter ${tag?'':'active'}" data-tag="" aria-pressed="${!tag}">Todas las personas <span>${view.graph.nodes.length}</span></button>` + [...all].sort(([a],[b])=>a.localeCompare(b,'es')).map(([name,ids])=>`<button class="tag-filter ${tag===name?'active':''}" data-tag="${esc(name)}" aria-pressed="${tag===name}">${esc(name)} <span>${ids.size}</span></button>`).join('');
}

function svgElement(name, attributes={}, content) {
  const element=document.createElementNS('http://www.w3.org/2000/svg',name);
  for(const [key,value] of Object.entries(attributes)) element.setAttribute(key,String(value));
  if(content!==undefined)element.textContent=String(content);
  return element;
}
function renderGraph(nodes) {
  const ids=new Set(nodes.map(n=>n.id)), scene=$('scene');scene.replaceChildren();
  points=layoutGraph(view.graph).filter(p=>ids.has(p.id));
  const byId=new Map(points.map(p=>[p.id,p]));
  for(const edge of view.graph.edges) {
    const a=byId.get(edge.from_id),b=byId.get(edge.to_id);if(!a || !b)continue;
    const status=standing(edge.standing,edge.basis),dx=b.x-a.x,sign=dx>=0?1:-1;
    const x1=a.x+sign*107,x2=b.x-sign*107;
    const path=dx===0?`M ${a.x+107} ${a.y} C ${a.x+165} ${a.y}, ${b.x+165} ${b.y}, ${b.x+107} ${b.y}`:`M ${x1} ${a.y} C ${(x1+x2)/2} ${a.y}, ${(x1+x2)/2} ${b.y}, ${x2} ${b.y}`;
    const line=svgElement('path',{d:path,class:`edge ${status.key}`});
    line.append(svgElement('title',{},`${a.label} · ${b.label}: ${labelFor(edge.kind)} · ${status.label}`));scene.append(line);
    if(dx!==0)scene.append(svgElement('text',{x:(x1+x2)/2,y:(a.y+b.y)/2-7,'text-anchor':'middle',class:`edge-label ${status.key}`},labelFor(edge.kind)));
  }
  if(points.length)scene.append(svgElement('text',{x:0,y:-90,'text-anchor':'middle',class:'graph-caption'},'TU CÍRCULO'));
  for (const node of points) {
    const isOwner=node.id===view.graph.owner_id;
    const group=svgElement('g',{transform:`translate(${node.x},${node.y})`,class:`graph-node ${isOwner?'node-owner':''}`,role:'button',tabindex:'0','aria-label':`Ver perfil de ${node.label}`,'aria-pressed':String(selected===node.id),'data-id':node.id});
    group.append(svgElement('rect',{x:-107,y:-41,width:214,height:82,rx:12,class:`node-card ${selected===node.id?'selected':''}`}));
    group.append(svgElement('circle',{cx:-77,cy:-4,r:19,fill:isOwner?'#cadcc2':'#e8efe2'}));
    group.append(svgElement('text',{x:-77,y:1,'text-anchor':'middle',class:'node-avatar'},initials(node.label)));
    const clipped=[...node.label].length>19?[...node.label].slice(0,17).join('')+'…':node.label;
    group.append(svgElement('text',{x:-47,y:-11,class:'node-name'},clipped));
    group.append(svgElement('text',{x:-47,y:7,class:'node-sub'},isOwner?'Tu punto de partida':node.degree===null?'Vínculo por revisar':node.degree===1?'En tu círculo cercano':`${node.degree} relaciones de distancia`));
    const tags=node.tags.filter(t=>!t.redacted && t.value).map(t=>t.value).slice(0,2).join(' · ');
    group.append(svgElement('text',{x:-47,y:24,class:'node-tag'},tags || (node.unresolved?'Sin camino declarado':isOwner?'Contexto personal':'')));
    scene.append(group);
  }
  transform();
}
function transform() {$('scene').setAttribute('transform',`translate(${offset.x},${offset.y}) scale(${scale})`);$('zoom-label').textContent=`${Math.round(scale*100)}%`;}
function fit() {
  const bounds=$('graph').getBoundingClientRect();if(!points.length || !bounds.width)return;
  const minX=Math.min(...points.map(p=>p.x))-125,maxX=Math.max(...points.map(p=>p.x))+125,minY=Math.min(...points.map(p=>p.y))-100,maxY=Math.max(...points.map(p=>p.y))+95;
  scale=Math.min(1.05,Math.max(.08,Math.min((bounds.width-35)/(maxX-minX),(bounds.height-80)/(maxY-minY))));
  offset={x:bounds.width/2-(minX+maxX)*scale/2,y:(bounds.height-25)/2-(minY+maxY)*scale/2};transform();
}
function zoom(factor,x=$('graph').clientWidth/2,y=$('graph').clientHeight/2) {
  const next=Math.max(.08,Math.min(2.5,scale*factor));offset={x:x-(x-offset.x)*next/scale,y:y-(y-offset.y)*next/scale};scale=next;transform();
}

function renderProfile(node) {
  if(!node){$('profile').innerHTML='<div class="profile-placeholder"><div class="profile-placeholder-icon" aria-hidden="true">◎</div><h2>Cada persona, su contexto</h2><p>Selecciona una persona para ver sus relaciones, preferencias y evidencias.</p></div>';return;}
  const labels=names(),relations=view.graph.edges.filter(e=>e.from_id===node.id || e.to_id===node.id);
  const tags=node.tags.filter(t=>!t.redacted).map(t=>{const state=standing(null,t.basis);return `<span class="chip ${state.key}" title="${esc(state.label)}">${esc(t.value)}${state.key!=='asserted'?' · ?':''}</span>`;}).join('');
  const profileTraits=node.traits.filter(t=>t.key!=='wanda.tag');
  $('profile').innerHTML=`<div class="profile-title"><div class="profile-avatar" aria-hidden="true">${esc(initials(node.label))}</div><h2>${esc(node.label)}</h2><p>${node.id===view.graph.owner_id?'El centro de tu contexto':node.degree===null?'Vínculo pendiente de confirmar':node.degree===1?'En tu círculo cercano':`${node.degree} relaciones de distancia`}</p><div class="profile-tags">${tags}</div></div>
    <section class="profile-section"><h3>Preferencias y contexto</h3><div class="trait-list">${profileTraits.length?profileTraits.map(t=>{const state=standing(null,t.basis);return `<div><span class="trait-key">${esc(t.key.replaceAll('_',' '))}</span><span class="trait-value">${t.redacted?'Dato reservado':esc(t.value || 'Sin información')}</span> <span class="status ${state.key}">${esc(state.label)}</span>${t.source_note?`<div class="source-note">${esc(t.source_note)}</div>`:''}</div>`;}).join(''):'<p class="muted">Sin preferencias registradas.</p>'}</div></section>
    <section class="profile-section"><h3>Relaciones</h3>${relations.length?relations.map(e=>{const other=e.from_id===node.id?e.to_id:e.from_id,state=standing(e.standing,e.basis);return `<button class="profile-relation" data-person="${esc(other)}"><span class="relation-initial" aria-hidden="true">${esc(initials(labels.get(other)))}</span><span><strong>${esc(labels.get(other) || 'Persona')}</strong><small>${esc(labelFor(e.kind))} · ${esc(state.label)}</small></span></button>${e.source_note?`<p class="source-note">${esc(e.source_note)}</p>`:''}`;}).join(''):'<p class="muted">Sin relaciones disponibles.</p>'}</section>
    <section class="profile-section"><h3>Camino hasta ti</h3><p class="profile-path">${node.path_to_owner.length?node.path_to_owner.map(id=>esc(labels.get(id) || id)).join(' → '):'No hay un camino declarado al propietario.'}</p></section>`;
}
function renderSources() {
  $('sources').innerHTML=view.sources.length?view.sources.map(s=>`<article class="source-card"><strong><span class="source-dot" aria-hidden="true"></span>${esc(text(s.source).replaceAll('_',' ') || 'Fuente')}</strong><p>${esc(text(s.device_id))}<br>Cobertura: ${esc(text(s.coverage) || 'No informada')}<br>Última observación: ${esc(date(s.last_observed_at_ms))}</p></article>`).join(''):'<p class="muted">No hay fuentes configuradas. La vista no supone que se esté capturando contenido.</p>';
  $('updated').textContent=demo?'Fixture · sin sincronización':view.generated_at_ms?`Vista del ${date(view.generated_at_ms)}`:'Sin sincronizar';
}
function render() {
  const focusedPerson=document.activeElement?.getAttribute('data-id');
  const nodes=visible();selected=selectedNode(nodes,selected)?.id || null;
  $('demo-banner').hidden=!demo;
  $('question').disabled=demo || !token || !view.domain || busy;
  $('ask').disabled=$('question').disabled;
  $('refresh').disabled=demo || !token || !$('domain').value;
  $('person-count').textContent=`${nodes.length} ${nodes.length===1?'persona':'personas'}`;
  renderTags();renderGraph(nodes);renderProfile(selectedNode(nodes,selected));renderSources();
  $('people-list').innerHTML=nodes.length?nodes.map(n=>`<button class="person-card ${selected===n.id?'selected':''}" data-person="${esc(n.id)}" aria-pressed="${selected===n.id}"><span class="relation-initial" aria-hidden="true">${esc(initials(n.label))}</span><span><strong>${esc(n.label)}</strong><small>${esc(n.tags.filter(t=>!t.redacted).map(t=>t.value).join(' · ') || 'Sin etiquetas')}</small></span></button>`).join(''):'<p class="muted">No hay personas que coincidan con esta búsqueda.</p>';
  $('map-empty').hidden=nodes.length>0;
  if(!nodes.length) {
    const filtered=view.graph.nodes.length>0;
    $('map-empty').querySelector('h2').textContent=filtered?'No encontramos coincidencias':'Tu historia empieza contigo';
    $('map-empty').querySelector('p').textContent=filtered?'Prueba otro nombre o vuelve a todas las etiquetas.':token && !demo?'Este dominio todavía no tiene personas disponibles. Revisa la configuración y las fuentes de Wanda.':'Conecta Wanda para ver tu círculo, sus preferencias y las fuentes que les dan contexto.';
    $('map-empty').querySelector('.empty-actions').hidden=filtered || Boolean(token);
  }
  $('recommendations').innerHTML=view.recommendations.length?view.recommendations.map(r=>recommendationHTML(r)).join(''):'<div class="recommendation-empty">Las recomendaciones aparecerán aquí cuando haya evidencia suficiente.</div>';
  $('graph-area').hidden=mode!=='map';$('people-list').hidden=mode!=='list';
  for(const name of ['map','list']){$(`${name}-view`).classList.toggle('active',mode===name);$(`${name}-view`).setAttribute('aria-pressed',String(mode===name));}
  if(focusedPerson){const replacement=[...$('scene').children].find(el=>el.getAttribute('data-id')===focusedPerson);replacement?.focus({preventScroll:true});}
}

function select(id) {
  selected=selectedNode(view.graph.nodes,id)?.id || null;
  if(selected && !selectedNode(visible(),selected)) {tag='';$('search').value='';}
  render();
}
$('tags').addEventListener('click',event=>{const button=event.target.closest('[data-tag]');if(!button)return;tag=button.dataset.tag;render();fit();});
$('search').addEventListener('input',()=>{render();fit();});
for(const id of ['people-list','profile'])$(id).addEventListener('click',event=>{const button=event.target.closest('[data-person]');if(button)select(button.dataset.person);});
for(const name of ['map','list'])$(`${name}-view`).addEventListener('click',()=>{mode=name;render();if(mode==='map')fit();});
$('graph').addEventListener('click',event=>{const node=event.target.closest('[data-id]');if(node && event.detail===0)select(node.dataset.id);});
$('graph').addEventListener('keydown',event=>{
  const node=event.target.closest('[data-id]');
  if(node && ['Enter',' '].includes(event.key)){event.preventDefault();select(node.dataset.id);return;}
  const delta={ArrowLeft:[30,0],ArrowRight:[-30,0],ArrowUp:[0,30],ArrowDown:[0,-30]}[event.key];
  if(delta){event.preventDefault();offset.x+=delta[0];offset.y+=delta[1];transform();}
  if(event.key==='+' || event.key==='='){event.preventDefault();zoom(1.2);}
  if(event.key==='-'){event.preventDefault();zoom(1/1.2);}
  if(event.key==='0'){event.preventDefault();fit();}
});
$('graph').addEventListener('pointerdown',event=>{if(event.button!==0)return;dragging={id:event.pointerId,nodeId:event.target.closest('[data-id]')?.dataset.id,x:event.clientX,y:event.clientY,startX:event.clientX,startY:event.clientY,moved:false};$('graph').setPointerCapture(event.pointerId);});
$('graph').addEventListener('pointermove',event=>{if(!dragging || event.pointerId!==dragging.id)return;const dx=event.clientX-dragging.x,dy=event.clientY-dragging.y;dragging.moved ||= Math.hypot(event.clientX-dragging.startX,event.clientY-dragging.startY)>4;offset.x+=dx;offset.y+=dy;dragging.x=event.clientX;dragging.y=event.clientY;if(dragging.moved)$('graph').classList.add('dragging');transform();});
function stopDrag(event){if(!dragging)return;$('graph').classList.remove('dragging');const ended=dragging;if(event.type==='pointerup' && !ended.moved && ended.nodeId)select(ended.nodeId);setTimeout(()=>{if(dragging===ended)dragging=null;},0);}
$('graph').addEventListener('pointerup',stopDrag);$('graph').addEventListener('pointercancel',stopDrag);
$('graph').addEventListener('wheel',event=>{if(!event.ctrlKey && !event.metaKey)return;event.preventDefault();const rect=$('graph').getBoundingClientRect();zoom(event.deltaY<0?1.12:1/1.12,event.clientX-rect.left,event.clientY-rect.top);},{passive:false});
$('zoom-in').addEventListener('click',()=>zoom(1.2));$('zoom-out').addEventListener('click',()=>zoom(1/1.2));$('fit').addEventListener('click',fit);
new ResizeObserver(()=>{if(mode==='map')fit();}).observe($('graph-area'));
document.addEventListener('keydown',event=>{if(event.key==='/' && !['INPUT','TEXTAREA','SELECT'].includes(document.activeElement?.tagName) && !$('pair-dialog').open){event.preventDefault();$('search').focus();}});

function showPair(){$('pair-error').textContent='';$('pair-dialog').showModal();$('token').focus();}
$('pair-button').addEventListener('click',showPair);$('empty-connect').addEventListener('click',showPair);$('close-pair').addEventListener('click',()=>$('pair-dialog').close());
$('pair-form').addEventListener('submit',async event=>{event.preventDefault();auth($('token').value.trim());$('token').value='';$('pair-error').textContent='';try{await connect();$('pair-dialog').close();}catch(err){$('pair-error').textContent=errorCopy(err);}});
$('forget-token').addEventListener('click',()=>{++generation;pending?.abort();querying?.abort();auth('');demo=false;busy=false;view=normalizeView(null);selected=null;tag='';setDomains([]);$('connection').textContent='Sin conectar';$('pair-button').textContent='Conectar';$('question').value='';$('query-state').textContent='El contexto privado se consulta con un modelo local elegible.';message('');render();$('pair-dialog').close();});
$('domain').addEventListener('change',()=>{$('question').value='';$('query-state').textContent='El contexto privado se consulta con un modelo local elegible.';load();});$('refresh').addEventListener('click',load);
$('demo').addEventListener('click',async()=>{const {demoView}=await import('./demo.mjs');++generation;pending?.abort();querying?.abort();demo=true;busy=false;view=normalizeView(demoView());selected=view.graph.owner_id;tag='';$('search').value='';setDomains([{domain:view.domain,label:'Contexto ficticio de demostración'}],view.domain);$('connection').textContent='Demostración';$('query-state').textContent='La demo no ejecuta inferencia ni consultas reales. Conecta tu contexto para preguntar a Wanda.';message('');render();fit();});
$('leave-demo').addEventListener('click',()=>{demo=false;if(token){connect().catch(()=>{});}else{view=normalizeView(null);selected=null;tag='';setDomains([]);$('connection').textContent='Sin conectar';render();}});

$('query-form').addEventListener('submit',async event=>{
  event.preventDefault();const question=$('question').value.trim(),domain=view.domain;
  if(!question || busy || demo || !domain)return;
  const mine=generation;querying?.abort();querying=new AbortController();busy=true;render();$('query-state').textContent='Revisando el contexto y sus evidencias…';
  try {
    const result=await api('/v1/query',{method:'POST',body:JSON.stringify({domain,question,deep:$('deep').checked}),signal:querying.signal});
    if(mine!==generation)return;
    view.recommendations=list(result.recommendations).filter(r=>r && typeof r==='object');
    const state=text(result.state);
    const messages={missing_model:'No hay un modelo local elegible para esta consulta.',unconfigured:'Este dominio todavía no está configurado.',insufficient_evidence:'No hay evidencia suficiente para producir una propuesta.',refused:'La consulta fue rechazada por las reglas del contexto.',no_recommendation:'La evidencia no justifica una recomendación.'};
    $('query-state').textContent=messages[state] || (view.recommendations.length?`Propuestas listas para revisar${text(result.model)?` · modelo ${text(result.model)}`:''}.`:'No se produjo una recomendación. Revisa las evidencias o reformula la consulta.');
  }catch(err){if(err.name!=='AbortError' && mine===generation)$('query-state').textContent=errorCopy(err);}
  finally{if(mine===generation){busy=false;render();}}
});

render();
if(token)connect().catch(()=>{});
