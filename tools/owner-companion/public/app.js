const $=id=>document.getElementById(id);
let additionApproval;
const requestId=location.hash.startsWith('#request=')?location.hash.slice(9):null;
history.replaceState(null,'',location.pathname);
async function api(path,body={}){const response=await fetch('/api/'+path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});const data=await response.json();if(!response.ok)throw Error(data.error);return data;}
async function status(){const data=await api('status');$('identity').textContent=data.ownerName+' · nodo '+data.nodeId;$('approve').disabled=!data.registered;$('register').disabled=data.registered&&!additionApproval;}
async function ceremony(kind){
  $('register').disabled=true;$('approve').disabled=true;
  try{
    const action=kind==='authenticate'?{kind:$('kind').value,description:$('description').value,resource:$('resource').value}:undefined;
    if(action&&!window.confirm('Nodo y acción a aprobar:\n'+$('identity').textContent+'\n'+action.description+'\n'+action.resource))return;
    const issued=await api('options/'+kind,{action,requestId,approvalId:additionApproval,activationCode:$('activation').value});
    $('activation').value='';
    if(issued.action)$('status').textContent=issued.action.description+' · '+issued.action.resource;
    $('status').textContent='Continúa en el diálogo seguro del navegador. Puedes elegir Android o iPhone.';
    const response=kind==='register'?await SimpleWebAuthnBrowser.startRegistration({optionsJSON:issued.options}):await SimpleWebAuthnBrowser.startAuthentication({optionsJSON:issued.options});
    const result=await api('verify/'+kind,{response});
    if(result.action?.kind==='add-device')additionApproval=result.approvalId;
    $('status').textContent=kind==='register'?'Passkey vinculada al nodo.':'Aprobación verificada: '+result.action.description+' · caduca en 60 segundos. No se ejecutó ninguna acción.';
  }catch(error){$('status').textContent=error.message;}finally{await status();}
}
$('register').onclick=()=>ceremony('register');$('approve').onclick=()=>ceremony('authenticate');status().then(async()=>{
  if(requestId){const request=await api('request',{requestId});
    $('kind').value=request.action.kind;$('description').value=request.action.description;$('resource').value=request.action.resource;
    for(const id of ['kind','description','resource'])$(id).disabled=true;
  }
}).catch(error=>$('status').textContent=error.message);
