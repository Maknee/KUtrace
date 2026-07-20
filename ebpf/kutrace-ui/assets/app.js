const state={filters:[],views:[],full:null,range:null,cpuPage:0,cpuPageSize:64,flags:0,mipmapWidth:0,mipmapCoarseWidth:0,mipmapMaxBins:0,selectedAgent:null,selectedAgentName:'',selectedAgentSql:'',refreshController:null};
const $=id=>document.getElementById(id);
const colors={agent:'#d783ff',annotation:'#c78cff',syscall:'#53d6a5',kernel:'#ffb454',user:'#6ea8fe',scheduler:'#f07178',rpc:'#ff8f70',resource:'#57b8d9',lock:'#e6c36a',wakeup:'#e6c36a',sample:'#95e6cb',special:'#7d899c'};
const ipcValues=['0','1/8','1/4','3/8','1/2','5/8','3/4','7/8','1.0','1.25','1.5','1.75','2.0','2.5','3.0','3.5'];
const llcValues=['0','64','512','1KB','2KB','4KB','8KB','16K','32K','64K','128K','256K','512K','1M','2M','4M+'];

async function query(sql,limit=1000,signal=undefined){
  const response=await fetch('/api/query',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({sql,limit}),signal});
  const body=await response.json();
  if(!response.ok)throw new Error(body.error||response.statusText);
  return body;
}
function literal(value){return /^-?\d+(\.\d+)?$/.test(value)?value:`'${value.replaceAll("'","''")}'`}
function filterSql(){return state.filters.map(f=>f.op==='contains'?`${f.field} LIKE '%' || ${literal(f.value)} || '%'`:`${f.field} ${f.op} ${literal(f.value)}`).join(' AND ')}
function where(extra=''){const parts=[filterSql(),extra].filter(Boolean);return parts.length?'WHERE '+parts.join(' AND '):''}
function renderTable(target,result){
  const head=`<thead><tr>${result.columns.map(c=>`<th>${escapeHtml(c)}</th>`).join('')}</tr></thead>`;
  const body=result.rows.map(r=>`<tr>${r.map(v=>`<td>${escapeHtml(v??'')}</td>`).join('')}</tr>`).join('');
  target.innerHTML=head+`<tbody>${body}</tbody>`;
}
function escapeHtml(value){const e=document.createElement('span');e.textContent=String(value);return e.innerHTML}
function performance(raw){
  if(!raw)return '';
  const hasIpc=(state.flags&128)!==0,hasLlc=(state.flags&32)!==0;
  if(hasIpc&&hasLlc)return `IPC ${ipcValues[(raw&12)+1]} · LLC ${llcValues[[0,2,5,8][raw&3]]}`;
  if(hasIpc)return `IPC ${ipcValues[raw&15]}`;
  if(hasLlc)return `LLC ${llcValues[raw&15]}`;
  return `sample ${raw}`;
}
function updatePerformanceLegend(){
  const parts=[];if(state.flags&128)parts.push('<span class="perf-ipc">◆ IPC</span>');if(state.flags&32)parts.push('<span class="perf-llc">◆ LLC</span>');$('perf-legend').innerHTML=parts.join(' ');
}
function updatePrintSummary(){
  $('print-trace-title').textContent=$('trace-title').textContent||'Untitled capture';
  $('print-range').textContent=state.range?`Visible range: ${state.range[0].toFixed(6)}s – ${state.range[1].toFixed(6)}s (${(state.range[1]-state.range[0]).toFixed(6)}s)`:'Visible range unavailable';
  $('print-filters').textContent=state.filters.length?`Filters: ${state.filters.map(f=>`${f.field} ${f.op} ${f.value}`).join(' · ')}`:'Filters: none';
}
function renderChips(){
  $('filter-chips').innerHTML=state.filters.map((f,i)=>`<span class="chip">${escapeHtml(f.field)} ${escapeHtml(f.op)} ${escapeHtml(f.value)}<button data-remove="${i}">×</button></span>`).join('');
  document.querySelectorAll('[data-remove]').forEach(b=>b.onclick=()=>{state.filters.splice(+b.dataset.remove,1);renderChips();refresh()});
  updatePrintSummary();
}
function renderSavedViews(){
  const target=$('saved-views');
  if(!state.views.length){target.className='saved-views muted';target.textContent='No saved SQL views.';return}
  target.className='saved-views';
  target.innerHTML=state.views.map((view,index)=>`<div class="saved-view-row"><button data-load-view="${index}" title="Load and run ${escapeHtml(view.name)}"><span>${escapeHtml(view.name)}</span></button><button data-delete-view="${index}" title="Delete ${escapeHtml(view.name)}" aria-label="Delete ${escapeHtml(view.name)}">×</button></div>`).join('');
  document.querySelectorAll('[data-load-view]').forEach(button=>button.onclick=()=>{const view=state.views[+button.dataset.loadView];$('sql').value=view.sql;$('view-name').value=view.name;runSql()});
  document.querySelectorAll('[data-delete-view]').forEach(button=>button.onclick=()=>{state.views.splice(+button.dataset.deleteView,1);renderSavedViews()});
}
function workspaceState(){return {kind:'kutrace-workspace',version:2,filters:state.filters,sql:$('sql').value,range:state.range,views:state.views}}
function validWorkspace(value){
  const fields=new Set(['category','cpu','pid','event','rpc','name']),ops=new Set(['=','!=','contains','>=','<=']);
  if(!value||typeof value!=='object'||(value.kind&&value.kind!=='kutrace-workspace')||(value.version&&![1,2].includes(value.version)))throw new Error('Unsupported workspace file');
  const filters=Array.isArray(value.filters)?value.filters:[];
  const views=Array.isArray(value.views)?value.views:[];
  if(filters.length>64||filters.some(f=>!f||!fields.has(f.field)||!ops.has(f.op)||typeof f.value!=='string'||f.value.length>256))throw new Error('Invalid workspace filters');
  if(views.length>32||views.some(view=>!view||typeof view.name!=='string'||!view.name.trim()||view.name.length>80||typeof view.sql!=='string'||!view.sql.trim()||view.sql.length>100000)||new Set(views.map(view=>view.name)).size!==views.length)throw new Error('Invalid saved SQL views');
  if(views.reduce((size,view)=>size+view.name.length+view.sql.length,0)>250000)throw new Error('Saved SQL views are too large');
  if(value.sql!==undefined&&(typeof value.sql!=='string'||value.sql.length>100000))throw new Error('Invalid workspace SQL');
  if(value.range!==undefined&&(!Array.isArray(value.range)||value.range.length!==2||value.range.some(v=>!Number.isFinite(v))))throw new Error('Invalid workspace range');
  return {filters,views:views.map(view=>({name:view.name.trim(),sql:view.sql})),sql:value.sql,range:value.range};
}
function applyWorkspace(value){
  const workspace=validWorkspace(value);state.filters=workspace.filters;state.views=workspace.views;if(workspace.sql!==undefined)$('sql').value=workspace.sql;
  if(workspace.range){const a=Math.max(state.full[0],workspace.range[0]),b=Math.min(state.full[1],workspace.range[1]);state.range=a<b?[a,b]:[...state.full]}
  renderChips();renderSavedViews();
}
async function loadMetadata(){
  const meta=await query("SELECT key,value FROM metadata WHERE key IN ('title','tracebase','flags','timeline_mipmap_width','timeline_mipmap_coarse_width','timeline_mipmap_max_bins')",10);
  const values=Object.fromEntries(meta.rows);$('trace-title').textContent=[values.title,values.tracebase].filter(Boolean).join(' · ');state.flags=Number(values.flags||0);state.mipmapWidth=Number(values.timeline_mipmap_width||0);state.mipmapCoarseWidth=Number(values.timeline_mipmap_coarse_width||0);state.mipmapMaxBins=Number(values.timeline_mipmap_max_bins||0);updatePerformanceLegend();
  const extent=await query('SELECT MIN(ts),MAX(ts_end) FROM events',1);state.full=extent.rows[0].slice(0,2);state.range=[...state.full];
  updatePrintSummary();
}
function boundedRange(start,end){
  const [fullStart,fullEnd]=state.full,fullSpan=fullEnd-fullStart,span=end-start;
  if(!Number.isFinite(span)||span<=0||span>=fullSpan)return [fullStart,fullEnd];
  if(start<fullStart){end+=fullStart-start;start=fullStart}
  if(end>fullEnd){start-=end-fullEnd;end=fullEnd}
  return [Math.max(fullStart,start),Math.min(fullEnd,end)];
}
function setRange(start,end){state.range=boundedRange(start,end);return refresh()}
function zoomRange(factor){const [start,end]=state.range,center=(start+end)/2,span=(end-start)*factor;return setRange(center-span/2,center+span/2)}
function panRange(fraction){const [start,end]=state.range,delta=(end-start)*fraction;return setRange(start+delta,end+delta)}
function installTimelineNavigation(canvas,width){
  const plotStart=48,plotWidth=Math.max(1,width-plotStart),selection=$('time-selection');let dragStart=null;
  const plotX=event=>Math.max(0,Math.min(plotWidth,event.offsetX-plotStart));
  const timeAt=x=>state.range[0]+(state.range[1]-state.range[0])*x/plotWidth;
  const hideSelection=()=>{selection.style.display='none';dragStart=null};
  canvas.onpointerdown=event=>{
    if(event.button!==0||event.offsetX<plotStart)return;
    dragStart=plotX(event);selection.style.display='block';selection.style.left=`${plotStart+dragStart}px`;selection.style.width='0px';canvas.setPointerCapture(event.pointerId);event.preventDefault();
  };
  canvas.onpointermove=event=>{
    if(dragStart===null)return;const current=plotX(event),left=Math.min(dragStart,current);selection.style.left=`${plotStart+left}px`;selection.style.width=`${Math.abs(current-dragStart)}px`;
  };
  canvas.onpointercancel=hideSelection;
  canvas.onpointerup=event=>{
    if(dragStart===null)return;const finish=plotX(event),begin=dragStart;hideSelection();
    if(Math.abs(finish-begin)>=4)setRange(timeAt(Math.min(begin,finish)),timeAt(Math.max(begin,finish)));
    else{const center=timeAt(finish),span=(state.range[1]-state.range[0])/4;setRange(center-span/2,center+span/2)}
  };
  canvas.ondblclick=event=>{event.preventDefault();setRange(...state.full)};
  canvas.onwheel=event=>{
    if(event.ctrlKey||event.metaKey){
      event.preventDefault();const fraction=plotX(event)/plotWidth,[rangeStart,rangeEnd]=state.range,span=rangeEnd-rangeStart,nextSpan=span*Math.exp(event.deltaY*.002),anchor=rangeStart+span*fraction;setRange(anchor-nextSpan*fraction,anchor+nextSpan*(1-fraction));
    }else if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY)){
      event.preventDefault();const delta=(event.deltaX||event.deltaY)*(state.range[1]-state.range[0])/plotWidth;setRange(state.range[0]+delta,state.range[1]+delta);
    }
  };
}
function exactTimelineSql(start,end,bucket,bucketCount,cpuScope){return `WITH RECURSIVE scoped_events(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT cpu,event,name,category,ipc,ts,ts_end,
           MAX(0,CAST((ts-${start})/${bucket} AS INTEGER)),
           MIN(${bucketCount-1},CAST((ts_end-${start})/${bucket} AS INTEGER))
      FROM events ${where(`ts < ${end} AND ts_end > ${start} AND ${cpuScope}`)}
  ), expanded(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT * FROM scoped_events
    UNION ALL
    SELECT cpu,event,name,category,ipc,ts,ts_end,bucket+1,last_bucket
      FROM expanded WHERE bucket<last_bucket
  ), scored AS (
    SELECT bucket,cpu,event,name,category,ipc,
           SUM(MIN(ts_end,${start}+(bucket+1)*${bucket})-MAX(ts,${start}+bucket*${bucket})) weight,
           COUNT(*) count
      FROM expanded
     WHERE ts<${start}+(bucket+1)*${bucket} AND ts_end>${start}+bucket*${bucket}
     GROUP BY bucket,cpu,event,name,category,ipc),
  ranked AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket,cpu ORDER BY weight DESC,count DESC,event,name,category,ipc) rank FROM scored)
  SELECT bucket,cpu,event,name,category,ipc,weight,count FROM ranked WHERE rank=1 ORDER BY cpu,bucket`}
function mipmapTimelineSql(start,end,bucket,bucketCount,cpuScope,table){
  const maxDuration=state.mipmapWidth*state.mipmapMaxBins,mipmapScope=[cpuScope,filterSql()].filter(Boolean).join(' AND ');
  return `WITH RECURSIVE mapped(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
    SELECT MAX(0,CAST((bucket_start-${start})/${bucket} AS INTEGER)),
           MIN(${bucketCount-1},CAST((bucket_end-${start})/${bucket} AS INTEGER)),
           cpu,event,name,category,ipc,bucket_start,bucket_end,weight,count
      FROM ${table}
     WHERE bucket_start<${end} AND bucket_end>${start} AND ${mipmapScope}
  ), map_expanded(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
    SELECT * FROM mapped
    UNION ALL
    SELECT bucket+1,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count
      FROM map_expanded WHERE bucket<last_bucket
  ), long_events(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT cpu,event,name,category,ipc,ts,ts_end,
           MAX(0,CAST((ts-${start})/${bucket} AS INTEGER)),
           MIN(${bucketCount-1},CAST((ts_end-${start})/${bucket} AS INTEGER))
      FROM events
     WHERE (dur=0 OR dur>${maxDuration}) AND ${mipmapScope}
       AND ((dur=0 AND ts>=${start} AND ts<${end}) OR (dur>0 AND ts<${end} AND ts_end>${start}))
  ), long_expanded(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT * FROM long_events
    UNION ALL
    SELECT cpu,event,name,category,ipc,ts,ts_end,bucket+1,last_bucket
      FROM long_expanded WHERE bucket<last_bucket
  ), combined AS (
    SELECT bucket,cpu,event,name,category,ipc,
           weight*MAX(0,MIN(source_end,${end},${start}+(bucket+1)*${bucket})-MAX(source_start,${start},${start}+bucket*${bucket}))/(source_end-source_start) weight,
           count
      FROM map_expanded
    UNION ALL
    SELECT bucket,cpu,event,name,category,ipc,
           MAX(0,MIN(ts_end,${end},${start}+(bucket+1)*${bucket})-MAX(ts,${start},${start}+bucket*${bucket})) weight,
           1 count
      FROM long_expanded
  ), scored AS (
    SELECT bucket,cpu,event,name,category,ipc,SUM(weight) weight,SUM(count) count
      FROM combined GROUP BY bucket,cpu,event,name,category,ipc
  ), ranked AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket,cpu ORDER BY weight DESC,count DESC,event,name,category,ipc) rank FROM scored)
  SELECT bucket,cpu,event,name,category,ipc,weight,count FROM ranked WHERE rank=1 ORDER BY cpu,bucket`;
}
async function drawTimeline(signal){
  const canvas=$('timeline'),ratio=devicePixelRatio||1,width=canvas.getBoundingClientRect().width,plotWidth=Math.max(1,width-48),[start,end]=state.range;
  canvas.dataset.ready='false';
  $('range-label').textContent=`${start.toFixed(6)}s – ${end.toFixed(6)}s · ${(end-start).toFixed(6)}s`;
  updatePrintSummary();
  const scoped=where(`ts < ${end} AND ts_end > ${start}`);
  const cpuResult=await query(`SELECT DISTINCT cpu FROM events ${scoped} ORDER BY cpu LIMIT 10000`,10000,signal),allCpus=cpuResult.rows.map(row=>row[0]),pageCount=Math.max(1,Math.ceil(allCpus.length/state.cpuPageSize));
  state.cpuPage=Math.min(state.cpuPage,pageCount-1);
  const firstCpu=state.cpuPage*state.cpuPageSize,visibleCpus=allCpus.slice(firstCpu,firstCpu+state.cpuPageSize),cpuScope=visibleCpus.length?`cpu IN (${visibleCpus.join(',')})`:'0';
  $('cpu-controls').hidden=pageCount===1;
  $('previous-cpus').disabled=state.cpuPage===0;$('next-cpus').disabled=state.cpuPage>=pageCount-1;
  $('cpu-window').textContent=allCpus.length?`CPUs ${firstCpu+1}–${firstCpu+visibleCpus.length} of ${allCpus.length}`:'No CPU tracks';
  const bucketCount=Math.max(1,Math.min(Math.floor(plotWidth),Math.floor(8000/Math.max(1,visibleCpus.length)))),bucket=(end-start)/bucketCount,pixelPerBucket=plotWidth/bucketCount;
  const mipmapCompatible=state.filters.every(filter=>['category','cpu','event'].includes(filter.field)),useMipmap=mipmapCompatible&&state.mipmapWidth>0&&bucket>=state.mipmapWidth*4,useCoarse=useMipmap&&state.mipmapCoarseWidth>0&&bucket>=state.mipmapCoarseWidth*4,mipmapTable=useCoarse?'timeline_mipmap_coarse':'timeline_mipmap',sql=useMipmap?mipmapTimelineSql(start,end,bucket,bucketCount,cpuScope,mipmapTable):exactTimelineSql(start,end,bucket,bucketCount,cpuScope);
  canvas.dataset.source=useMipmap?'mipmap':'events';
  canvas.dataset.mipmapLevel=useMipmap?(useCoarse?'coarse':'fine'):'none';
  const result=await query(sql,10000,signal),cpus=visibleCpus,height=Math.max(100,Math.min(340,24+cpus.length*22)),rowH=Math.max(4,Math.min(22,(height-24)/Math.max(cpus.length,1))),cpuY=new Map(cpus.map((c,i)=>[c,18+i*rowH]));
  canvas.style.height=`${height}px`;canvas.width=Math.max(300,width*ratio);canvas.height=height*ratio;
  const ctx=canvas.getContext('2d');ctx.scale(ratio,ratio);
  ctx.fillStyle='#090c12';ctx.fillRect(0,0,width,height);ctx.font='10px ui-monospace';
  for(const cpu of cpus){const y=cpuY.get(cpu);ctx.fillStyle='#748096';ctx.fillText(`CPU ${cpu}`,4,y+rowH-2);ctx.strokeStyle='#1b2230';ctx.beginPath();ctx.moveTo(48,y+rowH);ctx.lineTo(width,y+rowH);ctx.stroke()}
  for(const [b,cpu,event,name,category,ipc] of result.rows){const x=48+b*pixelPerBucket,y=cpuY.get(cpu);ctx.fillStyle=colors[category]||colors.special;ctx.fillRect(x,y,Math.ceil(pixelPerBucket),Math.max(3,rowH-2));if(ipc){if(state.flags&128){ctx.fillStyle='#fff';ctx.fillRect(x,y,Math.max(1,Math.ceil(pixelPerBucket)),2)}if(state.flags&32){ctx.fillStyle='#ffc080';ctx.fillRect(x,y+Math.max(2,rowH-4),Math.max(1,Math.ceil(pixelPerBucket)),2)}}}
  installTimelineNavigation(canvas,width);
  canvas.dataset.ready='true';
}
async function loadEvents(signal){
  const [start,end]=state.range,sql=`SELECT ts,dur,cpu,pid,event,name,category,arg0,retval,ipc FROM events ${where(`ts < ${end} AND ts_end > ${start}`)} ORDER BY ts LIMIT 501`,result=await query(sql,501,signal);
  const rows=result.rows.slice(0,500).map(row=>[...row,performance(row[9])]);renderTable($('event-table'),{columns:[...result.columns,'performance'],rows});$('event-count').textContent=result.rows.length>500?'500+ rows':`${result.rows.length} rows`;
}
async function loadAgentTree(signal){
  const [start,end]=state.range,sql=`WITH RECURSIVE tree(id,span_id,parent_span_id,name,ts,dur,depth,path) AS (
    SELECT id,span_id,parent_span_id,name,ts,dur,0,printf('%020d',id) FROM agent_spans
      WHERE parent_span_id=0 AND ts<${end} AND ts_end>${start}
    UNION ALL SELECT child.id,child.span_id,child.parent_span_id,child.name,child.ts,child.dur,tree.depth+1,tree.path||'.'||printf('%020d',child.id)
      FROM agent_spans child JOIN tree ON child.parent_span_id=tree.span_id WHERE tree.depth<32)
    SELECT depth,name,ROUND(ts,8),ROUND(dur*1000,3),CAST(span_id AS TEXT),CAST(parent_span_id AS TEXT),
           (SELECT COUNT(*) FROM agent_annotations annotation WHERE annotation.span_id=tree.span_id)
      FROM tree ORDER BY path LIMIT 1000`;
  const result=await query(sql,1000,signal),spanIds=new Set(result.rows.map(row=>row[4]));
  if(!spanIds.has(state.selectedAgent)){
    state.selectedAgent=result.rows.length?result.rows[0][4]:null;
    state.selectedAgentName=result.rows.length?result.rows[0][1]:'';
  }else{
    const selected=result.rows.find(row=>row[4]===state.selectedAgent);if(selected)state.selectedAgentName=selected[1];
  }
  $('agent-tree').innerHTML=result.rows.length?result.rows.map(row=>`<button class="agent-node${row[4]===state.selectedAgent?' selected':''}" style="padding-left:${4+row[0]*12}px" data-agent-span="${row[4]}"><span>${escapeHtml(row[1])}</span> ${row[3]}ms · #${row[4]}${row[6]?` · ${row[6]} note${row[6]===1?'':'s'}`:''}</button>`).join(''):'<span class="muted">No root agent spans in this range.</span>';
  document.querySelectorAll('[data-agent-span]').forEach(button=>button.onclick=async()=>{
    state.selectedAgent=button.dataset.agentSpan;
    state.selectedAgentName=button.querySelector('span').textContent;
    document.querySelectorAll('[data-agent-span]').forEach(node=>node.classList.toggle('selected',node===button));
    try{await loadAgentContext()}catch(error){$('query-status').textContent=error.message}
  });
  await loadAgentContext(signal);
}
async function loadAgentContext(signal){
  if(state.selectedAgent===null){
    state.selectedAgentSql='';$('open-agent-context').disabled=true;$('agent-context-title').textContent='Select an agent span';$('agent-context-count').textContent='';$('agent-context-sql').textContent='';$('agent-context-table').innerHTML='';$('agent-annotations').className='annotation-list muted';$('agent-annotations').textContent='No linked query, observation, decision, or result annotations.';return;
  }
  if(!/^-?\d+$/.test(state.selectedAgent))throw new Error('Invalid agent span id');
  const spanId=state.selectedAgent,sql=`WITH selected AS (
  SELECT id,span_id,name,ts,ts_end,pid FROM agent_spans WHERE span_id=${spanId} LIMIT 1
), context AS (
  SELECT 'annotation' AS source,a.ts,a.dur,a.pid,0 AS rpc,a.event,a.value AS arg0,a.span_id AS retval,a.annotation_kind AS category,a.name
    FROM agent_annotations a JOIN selected s ON a.span_id=s.span_id
  UNION ALL
  SELECT 'trace' AS source,e.ts,e.dur,e.pid,e.rpc,e.event,e.arg0,e.retval,e.category,e.name
    FROM events e JOIN selected s ON e.ts<s.ts_end AND e.ts_end>s.ts
   WHERE e.id!=s.id AND NOT(e.event BETWEEN 522 AND 525 AND e.retval=s.span_id)
)
SELECT source,ROUND(ts,8) AS ts,ROUND(dur*1000,3) AS duration_ms,pid,rpc,event,arg0,retval,category,name
  FROM context ORDER BY ts,source LIMIT 200`;
  state.selectedAgentSql=sql;$('open-agent-context').disabled=false;
  $('agent-context-sql').textContent=sql;
  const result=await query(sql,200,signal),annotations=result.rows.filter(row=>row[0]==='annotation');
  $('agent-context-title').textContent=`${state.selectedAgentName} · span #${spanId}`;
  $('agent-context-count').textContent=`${result.rows.length} context rows · ${annotations.length} annotations`;
  const annotationTarget=$('agent-annotations');
  if(annotations.length){
    annotationTarget.className='annotation-list';
    annotationTarget.innerHTML=annotations.map(row=>`<span class="annotation-pill"><b>${escapeHtml(row[8])}</b> ${escapeHtml(row[9])} · ${escapeHtml(row[6])}</span>`).join('');
  }else{
    annotationTarget.className='annotation-list muted';annotationTarget.textContent='No linked query, observation, decision, or result annotations.';
  }
  renderTable($('agent-context-table'),result);
}
async function loadRpcFlows(signal){
  const [start,end]=state.range,sql=`SELECT rpc_id,ROUND(MIN(ts),6),ROUND((MAX(ts_end)-MIN(ts))*1000,3),COUNT(*),GROUP_CONCAT(DISTINCT name) FROM rpc_activity ${where(`ts < ${end} AND ts_end > ${start}`)} GROUP BY rpc_id ORDER BY MIN(ts) LIMIT 200`,result=await query(sql,200,signal);
  $('rpc-flows').innerHTML=result.rows.length?result.rows.map(row=>`<div class="relation-row"><b>RPC ${row[0]}</b> · ${row[2]}ms · ${row[3]} events<br>${escapeHtml(row[4])}</div>`).join(''):'<span class="muted">No RPC activity in this range.</span>';
}
async function loadResourceActivity(signal){
  const [start,end]=state.range,sql=`SELECT ROUND(ts,6),ROUND(dur*1000,3),pid,rpc,event,arg0,name,category FROM resource_activity ${where(`ts < ${end} AND ts_end > ${start}`)} ORDER BY ts LIMIT 200`,result=await query(sql,200,signal);
  $('resource-activity').innerHTML=result.rows.length?result.rows.map(row=>`<div class="relation-row"><b>${escapeHtml(row[6])}</b> · ${row[1]}ms<br>${row[7]} #${row[5]} · PID ${row[2]}${row[3]?` · RPC ${row[3]}`:''}</div>`).join(''):'<span class="muted">No resource activity in this range.</span>';
}
async function refresh(){
  if(state.refreshController)state.refreshController.abort();const controller=new AbortController();state.refreshController=controller;
  try{await Promise.all([drawTimeline(controller.signal),loadEvents(controller.signal),loadAgentTree(controller.signal),loadRpcFlows(controller.signal),loadResourceActivity(controller.signal)])}
  catch(e){if(e.name!=='AbortError')$('query-status').textContent=e.message}
  finally{if(state.refreshController===controller)state.refreshController=null}
}
let sqlController=null;
async function runSql(sql=$('sql').value){
  if(sqlController)sqlController.abort();
  const controller=new AbortController();sqlController=controller;$('query-status').textContent='Running…';
  try{const r=await query(sql,1000,controller.signal);if(sqlController!==controller)return;renderTable($('query-table'),r);$('query-status').textContent=`${r.rows.length}${r.truncated?' (truncated)':''} rows · ${r.elapsed_ms.toFixed(2)} ms · exact SQL shown above`}
  catch(e){if(e.name!=='AbortError'&&sqlController===controller)$('query-status').textContent=e.message}
}

$('add-filter').onclick=()=>{const value=$('filter-value').value.trim();if(!value)return;state.filters.push({field:$('filter-field').value,op:$('filter-op').value,value});$('filter-value').value='';renderChips();refresh()};
$('run-sql').onclick=()=>runSql();$('show-schema').onclick=()=>{fetch('/api/schema').then(r=>r.json()).then(r=>{renderTable($('query-table'),r);$('query-status').textContent='sqlite_schema'})};
$('open-agent-context').onclick=()=>{if(!state.selectedAgentSql)return;$('sql').value=state.selectedAgentSql;$('view-name').value=`${state.selectedAgentName} context`;$('sql').scrollIntoView({behavior:'smooth',block:'center'});runSql()};
$('save-view').onclick=()=>{const name=$('view-name').value.trim(),sql=$('sql').value.trim();if(!name){$('query-status').textContent='Name the SQL view before saving';return}if(!sql){$('query-status').textContent='Cannot save an empty query';return}const existing=state.views.findIndex(view=>view.name===name);if(existing>=0)state.views[existing]={name,sql};else if(state.views.length>=32){$('query-status').textContent='Saved SQL view limit is 32';return}else state.views.push({name,sql});renderSavedViews();$('query-status').textContent=existing>=0?`Updated saved view “${name}”`:`Saved view “${name}”`};
$('reset-range').onclick=()=>setRange(...state.full);$('zoom-in').onclick=()=>zoomRange(.5);$('zoom-out').onclick=()=>zoomRange(2);$('pan-left').onclick=()=>panRange(-.25);$('pan-right').onclick=()=>panRange(.25);
$('previous-cpus').onclick=()=>{if(state.cpuPage>0){state.cpuPage--;refresh()}};$('next-cpus').onclick=()=>{state.cpuPage++;refresh()};
$('save-workspace').onclick=()=>{localStorage.setItem('kutrace-workspace',JSON.stringify(workspaceState()));$('query-status').textContent='Workspace saved locally'};
$('export-workspace').onclick=()=>{const url=URL.createObjectURL(new Blob([JSON.stringify(workspaceState(),null,2)],{type:'application/json'})),link=document.createElement('a');link.href=url;link.download='kutrace-workspace.json';link.click();URL.revokeObjectURL(url);$('query-status').textContent='Workspace exported'};
$('import-workspace').onclick=()=>$('workspace-file').click();
$('workspace-file').onchange=async event=>{try{const file=event.target.files[0];if(!file)return;applyWorkspace(JSON.parse(await file.text()));localStorage.setItem('kutrace-workspace',JSON.stringify(workspaceState()));await refresh();await runSql();$('query-status').textContent='Workspace imported'}catch(e){$('query-status').textContent=e.message}finally{event.target.value=''}};
window.addEventListener('keydown',event=>{
  if((event.target instanceof Element&&event.target.matches('input,textarea,select'))||event.ctrlKey||event.metaKey||event.altKey)return;
  const actions={'+':'zoom-in','=':'zoom-in','-':'zoom-out','0':'reset-range',Home:'reset-range',ArrowLeft:'pan-left','[':'pan-left',ArrowRight:'pan-right',']':'pan-right'},action=actions[event.key];
  if(action){event.preventDefault();$(action).click()}
});
let resizeTimer=null;window.addEventListener('resize',()=>{clearTimeout(resizeTimer);resizeTimer=setTimeout(()=>refresh(),100)});
(async()=>{await loadMetadata();const saved=JSON.parse(localStorage.getItem('kutrace-workspace')||'null');if(saved)applyWorkspace(saved);else{renderChips();renderSavedViews()}await refresh();await runSql()})().catch(e=>$('query-status').textContent=e.message);
