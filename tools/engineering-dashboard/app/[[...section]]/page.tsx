import {cookies} from "next/headers";
import {notFound,redirect} from "next/navigation";
import {authenticated,SESSION_COOKIE} from "../../lib/auth.ts";
import {getStore} from "../../lib/store.ts";
import type {StatusEvent} from "../../lib/events.ts";
import {checkKinds} from "../../lib/events.ts";
export const dynamic="force-dynamic";
export const runtime="nodejs";
const sections:Record<string,string>={"":"Overview",milestones:"Hitos",builds:"Builds",tests:"Tests",benchmarks:"Benchmarks",modules:"Módulos",repo:"Repositorio",git:"Git",runtime:"Runtime",ipc:"IPC",database:"Base de datos",replay:"Replay",security:"Seguridad",deployments:"Despliegues"};
const phaseFor:Record<string,string>={builds:"BUILD",tests:"TEST",benchmarks:"BENCHMARK",deployments:"DEPLOY",database:"MIGRATION",git:"COMMIT"};
function Badge({status}:{status:string}){return <span className={`badge ${status}`}>{status}</span>;}
function Timeline({events}:{events:StatusEvent[]}){return <div className="timeline">{events.map(event=><article key={event.event_id}>
  <div className="event-meta"><time dateTime={event.timestamp}>{new Date(event.timestamp).toLocaleString("es-CL",{timeZone:"UTC"})} UTC</time><span>{event.module}</span><Badge status={event.status}/></div>
  <h3>{event.summary}</h3>{event.command&&<pre>{event.command}</pre>}
  {event.metrics&&<dl className="metrics">{Object.entries(event.metrics).map(([key,value])=><div key={key}><dt>{key}</dt><dd>{value}</dd></div>)}</dl>}
  {event.duration_ms!==undefined&&<p className="muted">Duración: {event.duration_ms} ms · Salida: {event.exit_code??"sin resultado"}</p>}
  {event.next_action&&<p>Siguiente: {event.next_action}</p>}</article>)}</div>;}
export default async function Page({params,searchParams}:{params:Promise<{section?:string[]}>;searchParams:Promise<{q?:string}>}){
  let store;try{store=await getStore();}catch{return <main className="login"><h1>Dashboard no disponible</h1><p>Falta configurar el almacén de estado. No se ha cargado información del proyecto.</p></main>;}
  if(!await authenticated(store,(await cookies()).get(SESSION_COOKIE)?.value))redirect("/login");
  const section=(await params).section?.join("/")??"";
  if(!Object.hasOwn(sections,section))notFound();
  let events:StatusEvent[];try{events=await store.recent(200);}catch{return <main className="login"><h1>Almacén no disponible</h1><p>No hay un resultado verificado.</p></main>;}
  const query=String((await searchParams).q??"").slice(0,192).toLowerCase();
  const latest=events[0],git=events.find(e=>e.git)?.git;
  const visible=events.filter(e=>(!phaseFor[section]||e.phase===phaseFor[section])&&(!query||`${e.module} ${e.summary} ${e.command??""}`.toLowerCase().includes(query)));
  const files=events.find(e=>e.files)?.files??[];
  const modules=[...new Set(events.map(e=>e.module))];
  return <div className="shell"><aside><a className="brand" href="/">ARCHIE<span>ENGINEERING</span></a><nav aria-label="Secciones">{Object.entries(sections).map(([path,label])=><a aria-current={section===path?"page":undefined} href={`/${path}`} key={path}>{label}</a>)}</nav><form method="post" action="/api/logout"><button className="quiet">Cerrar sesión</button></form></aside>
    <main><header><div><p className="eyebrow">CONTROL DE IMPLEMENTACIÓN</p><h1>{sections[section]}</h1></div><a className="refresh" href={`/${section}`}>Actualizar</a></header>
      <div className="scorecard"><section><span>Commit</span><strong className="mono">{git?.commit.slice(0,10)??"Sin publicar"}</strong><small>{git?.branch??"Sin rama publicada"}</small></section>
        <section><span>Fase</span><strong>{latest?.phase??"Sin eventos"}</strong><small>{latest?.module??"Esperando al publicador local"}</small></section>
        <section><span>Último estado</span><strong><Badge status={latest?.status??"unverified"}/></strong><small>{latest?.timestamp??"Sin actualización"}</small></section></div>
      <form className="filter" method="get"><label htmlFor="q">Filtrar</label><input id="q" name="q" defaultValue={query} placeholder="Módulo, resultado o comando"/><button>Aplicar</button></form>
      {section==="repo"?<div className="table-wrap"><table><thead><tr><th>Ruta</th><th>Tipo</th><th>Git</th><th>Bytes</th></tr></thead><tbody>{files.filter(f=>f.path.toLowerCase().includes(query)).map((file,i)=><tr key={i}><td className="mono">{file.path}</td><td>{file.kind}</td><td>{file.state}</td><td>{file.size??"—"}</td></tr>)}</tbody></table>{!files.length&&<p>No se ha publicado un inventario.</p>}</div>:
      section==="modules"||section==="milestones"?<div className="table-wrap"><table className="release-matrix"><thead><tr><th>Módulo / alcance</th><th>Madurez</th>{checkKinds.map(kind=><th key={kind}>{kind}</th>)}</tr></thead><tbody>{modules.filter(m=>!query||m.toLowerCase().includes(query)).map(module=>{const e=events.find(e=>e.module===module&&e.checks)??events.find(e=>e.module===module)!;return <tr key={module}><td>{module}<p className="muted">{e.summary}</p></td><td>{e.maturity??"Sin evaluar"}</td>{checkKinds.map(kind=>{const check=e.checks?.[kind];return <td key={kind}>{check?.command?<details><summary><Badge status={check.status}/></summary><pre>{check.command}</pre></details>:<Badge status={check?.status??"unverified"}/>}</td>;})}</tr>;})}</tbody></table></div>:
      visible.length?<Timeline events={visible}/>:<div className="empty"><h2>Sin evidencia publicada</h2><p>Esta sección no tiene resultados verificados. Un espacio vacío no significa PASS.</p></div>}
      <footer>Vista de solo lectura · Sin controles de ejecución · Los resultados pertenecen al commit y al entorno indicados.</footer>
    </main></div>;
}
