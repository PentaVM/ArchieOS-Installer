// Explicit opt-in fixture. No price, attraction claim, or live source is fabricated.
const trait = (key,value,basis='owner_declared') => ({key,value,basis,sensitivity:'personal_private',source_note:'Fixture ficticio de demostración',redacted:false});
const person = (id,label,tag,preferences=[],degree=1) => ({id,label,kind:id==='owner'?'owner':'consented_person',degree,path_to_owner:id==='owner'?['owner']:['owner',id],tags:tag?[trait('wanda.tag',tag)]:[],traits:preferences.map(([key,value])=>trait(key,value))});
const edge = (id,kind) => ({from_id:'owner',to_id:id,kind,basis:'owner_declared',declarer_id:'owner',source_note:'Declaración ficticia para explorar la vista',standing:'asserted'});
export function demoView() {
  return {schema_version:1,domain:'personal/owner',generated_at_ms:null,availability:'demo',pending_events:0,
    graph:{schema_version:1,owner_id:'owner',nodes:[
      person('owner','Tú',null,[['Interés','Senderismo y fotografía']],0),
      person('elena','Elena','Familia',[['Interés','Jardinería'],['Preferencia','Café sin azúcar']]),
      person('marcos','Marcos','Familia',[['Interés','Música y lectura']]),
      person('sofia','Sofía','Familia',[['Interés','Cocina y cine']]),
      person('valentina','Valentina','Pareja',[['Flor favorita','Tulipanes'],['Plan','Una tarde tranquila juntos']]),
      person('mateo','Mateo','Amigos',[['Interés','Ciclismo'],['Preferencia','Comida vegetariana']]),
      person('lucia','Lucía','Amigos',[['Interés','Fotografía']]),
      person('tomas','Tomás','Trabajo',[['Rol','Jefatura'],['Preferencia','Revisar los avances con anticipación']]),
      person('ines','Inés','Trabajo',[['Rol','Compañera de equipo'],['Interés','Diseño y libros']])
    ],edges:[edge('elena','parent'),edge('marcos','parent'),edge('sofia','sibling'),edge('valentina','partner'),edge('mateo','friend'),edge('lucia','friend'),edge('tomas','manager'),edge('ines','colleague'),
      {from_id:'mateo',to_id:'lucia',kind:'acquaintance',basis:'inferred',declarer_id:'',source_note:'Hipótesis ficticia, no confirmada',standing:'hypothesis'}]},
    recommendations:[
      {id:'demo-gift',kind:'gift',title:'Un detalle que tiene sentido',explanation:'En este fixture, Valentina declaró que le gustan los tulipanes.',suggestion:'Podrías considerar tulipanes y una nota personal. Falta consultar disponibilidad y precio actual.',subject_ids:['valentina'],evidence_refs:['fixture:preference/tulipanes'],standing:'inferred',alternatives:[],expires_at_ms:null},
      {id:'demo-plan',kind:'commitment',title:'Dale espacio a ambos planes',explanation:'El fixture incluye una entrega el viernes a las seis y un encuentro a las siete. No hay una incompatibilidad confirmada.',suggestion:'Revisa cuánto tiempo necesitas para terminar y trasladarte antes de cambiar un compromiso.',subject_ids:['tomas','valentina'],evidence_refs:['fixture:commitment/entrega','fixture:commitment/encuentro'],standing:'inferred',alternatives:['Podría ser posible mantener ambos compromisos.'],expires_at_ms:null},
      {id:'demo-message',kind:'communication',title:'Un mensaje cercano, a tu manera',explanation:'Mateo mencionó el ciclismo como interés en esta demostración.',suggestion:'«¿Te tinca salir en bici este fin de semana? Podemos coordinar una ruta tranquila». Revisa el texto antes de usarlo.',subject_ids:['mateo'],evidence_refs:['fixture:preference/ciclismo'],standing:'inferred',alternatives:[],expires_at_ms:null}
    ],sources:[{device_id:'fixture-demo',source:'demostración',coverage:'fixture · sin conexión en vivo',last_observed_at_ms:null}]
  };
}
