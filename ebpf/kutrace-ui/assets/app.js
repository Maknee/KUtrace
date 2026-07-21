const state={
  filters:[],views:[],full:null,range:null,selection:null,selectedEvent:null,
  cpuPage:0,cpuPageSize:64,trackMode:'cpu_pid',flags:0,mipmapWidth:0,
  mipmapCoarseWidth:0,mipmapMaxBins:0,selectedAgent:null,selectedAgentName:'',
  selectedAgentSql:'',refreshController:null,overviewController:null,overviewKey:'',
  overviewRows:[],hitRegions:[],search:'',searchInvert:false,followTail:false,
  eventCount:0,activeDock:'details',activeView:'timeline',timelineRenderer:'kutrace',
  overlays:{marks:true,arcs:true,locks:true,frequency:true,ipc:true,samples:true,colorblind:false},
  groups:{cpu:true,process:true,kernel:true,agent:true}
};
const $=id=>document.getElementById(id);
const colors={agent:'#d783ff',annotation:'#c78cff',mark:'#c78cff',syscall:'#53d6a5',kernel:'#ffb454',user:'#6ea8fe',scheduler:'#f07178',rpc:'#ff8f70',resource:'#57b8d9',lock:'#e6c36a',wakeup:'#e6c36a',sample:'#95e6cb',special:'#7d899c'};
const colorblindColors={agent:'#cc79a7',annotation:'#cc79a7',mark:'#cc79a7',syscall:'#009e73',kernel:'#e69f00',user:'#56b4e9',scheduler:'#d55e00',rpc:'#0072b2',resource:'#56b4e9',lock:'#f0e442',wakeup:'#f0e442',sample:'#009e73',special:'#999'};
const ipcValues=['0','1/8','1/4','3/8','1/2','5/8','3/4','7/8','1.0','1.25','1.5','1.75','2.0','2.5','3.0','3.5'];
const llcValues=['0','64','512','1KB','2KB','4KB','8KB','16K','32K','64K','128K','256K','512K','1M','2M','4M+'];
const TIMELINE_LABEL_WIDTH=116;
const DETAIL_EVENT_LIMIT=8000;
const laneLabels=['user / agent','syscall','kernel / sched','markers / I/O'];

async function query(sql,limit=1000,signal=undefined){
  const response=await fetch('/api/query',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({sql,limit}),signal});
  const body=await response.json();
  if(!response.ok)throw new Error(body.error||response.statusText);
  return body;
}
function literal(value){return /^-?\d+(\.\d+)?$/.test(value)?value:`'${value.replaceAll("'","''")}'`}
function filterSql(){return state.filters.map(f=>f.op==='contains'?`${f.field} LIKE '%' || ${literal(f.value)} || '%'`:`${f.field} ${f.op} ${literal(f.value)}`).join(' AND ')}
function visibilitySql(){
  const hidden=[];
  if(!state.overlays.marks)hidden.push("category NOT IN ('mark','annotation')");
  if(!state.overlays.arcs)hidden.push("category NOT IN ('rpc','wakeup')");
  if(!state.overlays.locks)hidden.push("category!='lock'");
  if(!state.overlays.frequency)hidden.push('event!=521');
  if(!state.overlays.samples)hidden.push("category!='sample'");
  if(!state.groups.kernel)hidden.push("category NOT IN ('kernel','scheduler')");
  if(!state.groups.agent)hidden.push("category NOT IN ('agent','annotation')");
  if((state.trackMode==='cpu'&&!state.groups.cpu)||(state.trackMode==='pid'&&!state.groups.process)||(state.trackMode==='cpu_pid'&&!state.groups.cpu&&!state.groups.process))hidden.push('0');
  return hidden.join(' AND ');
}
function where(extra=''){const parts=[filterSql(),visibilitySql(),extra].filter(Boolean);return parts.length?'WHERE '+parts.join(' AND '):''}
function escapeHtml(value){const e=document.createElement('span');e.textContent=String(value);return e.innerHTML}
function renderTable(target,result){
  const head=`<thead><tr>${result.columns.map(c=>`<th>${escapeHtml(c)}</th>`).join('')}</tr></thead>`;
  const body=result.rows.map(r=>`<tr>${r.map(v=>`<td>${escapeHtml(v??'')}</td>`).join('')}</tr>`).join('');
  target.innerHTML=head+`<tbody>${body}</tbody>`;
}
function performance(raw){
  if(!raw||!state.overlays.ipc)return '';
  const hasIpc=(state.flags&128)!==0,hasLlc=(state.flags&32)!==0;
  if(hasIpc&&hasLlc)return `IPC ${ipcValues[(raw&12)+1]} · LLC ${llcValues[[0,2,5,8][raw&3]]}`;
  if(hasIpc)return `IPC ${ipcValues[raw&15]}`;
  if(hasLlc)return `LLC ${llcValues[raw&15]}`;
  return `sample ${raw}`;
}
function updatePerformanceLegend(){
  const parts=[];
  if(state.overlays.ipc&&state.flags&128)parts.push('<span class="perf-ipc">◆ IPC</span>');
  if(state.overlays.ipc&&state.flags&32)parts.push('<span class="perf-llc">◆ LLC</span>');
  $('perf-legend').innerHTML=parts.join(' ');
}
function updatePrintSummary(){
  $('print-trace-title').textContent=$('trace-title').textContent||'Untitled capture';
  $('print-range').textContent=state.range?`Visible range: ${state.range[0].toFixed(6)}s – ${state.range[1].toFixed(6)}s (${(state.range[1]-state.range[0]).toFixed(6)}s)`:'Visible range unavailable';
  $('print-filters').textContent=state.filters.length?`Filters: ${state.filters.map(f=>`${f.field} ${f.op} ${f.value}`).join(' · ')}`:'Filters: none';
}
function renderChips(){
  $('filter-chips').innerHTML=state.filters.map((f,i)=>`<span class="chip">${escapeHtml(f.field)} ${escapeHtml(f.op)} ${escapeHtml(f.value)}<button data-remove="${i}">×</button></span>`).join('');
  document.querySelectorAll('[data-remove]').forEach(b=>b.onclick=()=>{state.filters.splice(+b.dataset.remove,1);state.overviewKey='';renderChips();refresh()});
  updatePrintSummary();
}
function renderSavedViews(){
  const target=$('saved-views');
  if(!state.views.length){target.className='saved-views muted';target.textContent='No saved SQL views.';return}
  target.className='saved-views';
  target.innerHTML=state.views.map((view,index)=>`<div class="saved-view-row"><button data-load-view="${index}" title="Load and run ${escapeHtml(view.name)}"><span>${escapeHtml(view.name)}</span></button><button data-delete-view="${index}" title="Delete ${escapeHtml(view.name)}" aria-label="Delete ${escapeHtml(view.name)}">×</button></div>`).join('');
  document.querySelectorAll('[data-load-view]').forEach(button=>button.onclick=()=>{const view=state.views[+button.dataset.loadView];$('sql').value=view.sql;$('view-name').value=view.name;activateDock('sql');runSql()});
  document.querySelectorAll('[data-delete-view]').forEach(button=>button.onclick=()=>{state.views.splice(+button.dataset.deleteView,1);renderSavedViews()});
}
function workspaceState(){return {kind:'kutrace-workspace',version:2,filters:state.filters,sql:$('sql').value,range:state.range,views:state.views,trackMode:state.trackMode,timelineRenderer:state.timelineRenderer,overlays:state.overlays}}
function validWorkspace(value){
  const fields=new Set(['category','cpu','pid','event','rpc','name']),ops=new Set(['=','!=','contains','>=','<=']);
  if(!value||typeof value!=='object'||(value.kind&&value.kind!=='kutrace-workspace')||(value.version&&![1,2].includes(value.version)))throw new Error('Unsupported workspace file');
  const filters=Array.isArray(value.filters)?value.filters:[],views=Array.isArray(value.views)?value.views:[];
  if(filters.length>64||filters.some(f=>!f||!fields.has(f.field)||!ops.has(f.op)||typeof f.value!=='string'||f.value.length>256))throw new Error('Invalid workspace filters');
  if(views.length>32||views.some(view=>!view||typeof view.name!=='string'||!view.name.trim()||view.name.length>80||typeof view.sql!=='string'||!view.sql.trim()||view.sql.length>100000)||new Set(views.map(view=>view.name)).size!==views.length)throw new Error('Invalid saved SQL views');
  if(views.reduce((size,view)=>size+view.name.length+view.sql.length,0)>250000)throw new Error('Saved SQL views are too large');
  if(value.sql!==undefined&&(typeof value.sql!=='string'||value.sql.length>100000))throw new Error('Invalid workspace SQL');
  if(value.range!==undefined&&(!Array.isArray(value.range)||value.range.length!==2||value.range.some(v=>!Number.isFinite(v))))throw new Error('Invalid workspace range');
  const trackMode=['cpu','pid','cpu_pid'].includes(value.trackMode)?value.trackMode:'cpu',timelineRenderer=value.timelineRenderer==='lanes'?'lanes':'kutrace',overlays=value.overlays&&typeof value.overlays==='object'?value.overlays:{};
  return {filters,views:views.map(view=>({name:view.name.trim(),sql:view.sql})),sql:value.sql,range:value.range,trackMode,timelineRenderer,overlays};
}
function applyWorkspace(value){
  const workspace=validWorkspace(value);state.filters=workspace.filters;state.views=workspace.views;state.trackMode=workspace.trackMode;Object.assign(state.overlays,workspace.overlays);
  if(workspace.sql!==undefined)$('sql').value=workspace.sql;
  if(workspace.range&&state.full){const a=Math.max(state.full[0],workspace.range[0]),b=Math.min(state.full[1],workspace.range[1]);state.range=a<b?[a,b]:[...state.full]}
  $('track-mode').value=state.trackMode;activateTimelineRenderer(workspace.timelineRenderer);syncToggleUi();renderChips();renderSavedViews();
}
async function loadMetadata(){
  const meta=await query("SELECT key,value FROM metadata WHERE key IN ('title','tracebase','flags','timeline_mipmap_width','timeline_mipmap_coarse_width','timeline_mipmap_max_bins')",10),values=Object.fromEntries(meta.rows);
  $('trace-title').textContent=[values.title,values.tracebase].filter(Boolean).join(' · ');state.flags=Number(values.flags||0);state.mipmapWidth=Number(values.timeline_mipmap_width||0);state.mipmapCoarseWidth=Number(values.timeline_mipmap_coarse_width||0);state.mipmapMaxBins=Number(values.timeline_mipmap_max_bins||0);
  const extent=await query('SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events',1),row=extent.rows[0];state.eventCount=Number(row[0]||0);state.full=[Number(row[1]||0),Number(row[2]||row[1]||1)];if(state.full[1]<=state.full[0])state.full[1]=state.full[0]+.000001;state.range=[...state.full];
  $('live-status').textContent=`${state.eventCount} events · static`;updatePerformanceLegend();updatePrintSummary();
}
function boundedRange(start,end){
  const [fullStart,fullEnd]=state.full,fullSpan=fullEnd-fullStart,span=end-start;
  if(!Number.isFinite(span)||span<=0||span>=fullSpan)return [fullStart,fullEnd];
  if(start<fullStart){end+=fullStart-start;start=fullStart}if(end>fullEnd){start-=end-fullEnd;end=fullEnd}
  return [Math.max(fullStart,start),Math.min(fullEnd,end)];
}
function updateRangeChrome(){
  if(!state.range)return;const [start,end]=state.range;$('range-label').textContent=`${start.toFixed(6)}s – ${end.toFixed(6)}s · ${(end-start).toFixed(6)}s`;$('ruler-start').textContent=`${start.toFixed(6)}s`;$('ruler-center').textContent=`${((start+end)/2).toFixed(6)}s`;$('ruler-end').textContent=`${end.toFixed(6)}s`;updatePrintSummary();
}
function previewTimelineRange(previous,next){
  const canvas=$('timeline'),width=canvas.getBoundingClientRect().width,height=canvas.getBoundingClientRect().height;if(!canvas.width||!canvas.height||width<=TIMELINE_LABEL_WIDTH||height<=0)return;
  const [oldStart,oldEnd]=previous,[newStart,newEnd]=next,oldSpan=oldEnd-oldStart,newSpan=newEnd-newStart,overlapStart=Math.max(oldStart,newStart),overlapEnd=Math.min(oldEnd,newEnd);if(oldSpan<=0||newSpan<=0)return;
  const source=document.createElement('canvas');source.width=canvas.width;source.height=canvas.height;source.getContext('2d').drawImage(canvas,0,0);const ratio=canvas.width/width,label=TIMELINE_LABEL_WIDTH*ratio,plot=canvas.width-label,ctx=canvas.getContext('2d');ctx.save();ctx.setTransform(1,0,0,1,0,0);ctx.fillStyle=state.timelineRenderer==='kutrace'?'#fff':'#080b11';ctx.fillRect(label,0,plot,canvas.height);
  if(overlapEnd>overlapStart){const sx=label+(overlapStart-oldStart)*plot/oldSpan,sw=(overlapEnd-overlapStart)*plot/oldSpan,dx=label+(overlapStart-newStart)*plot/newSpan,dw=(overlapEnd-overlapStart)*plot/newSpan;ctx.drawImage(source,sx,0,sw,canvas.height,dx,0,dw,canvas.height)}
  ctx.restore();canvas.dataset.preview='true';canvas.dataset.ready='false';
}
function setRange(start,end){const previous=state.range?[...state.range]:null,next=boundedRange(start,end);if(previous)previewTimelineRange(previous,next);state.range=next;updateOverviewViewport();updateRangeChrome();return refresh()}
function zoomRange(factor,anchor=(state.range[0]+state.range[1])/2){const [start,end]=state.range,span=(end-start)*factor,fraction=(anchor-start)/(end-start);return setRange(anchor-span*fraction,anchor+span*(1-fraction))}
function panRange(fraction){const [start,end]=state.range,delta=(end-start)*fraction;return setRange(start+delta,end+delta)}
function setSelection(start,end,event=null){
  if(!Number.isFinite(start)||!Number.isFinite(end))return;
  if(end<start)[start,end]=[end,start];const minimum=Math.max((state.full[1]-state.full[0])*1e-9,1e-9);if(end-start<minimum)end=start+minimum;
  state.selection=[Math.max(state.full[0],start),Math.min(state.full[1],end)];state.selectedEvent=event;
  $('zoom-selection').disabled=false;$('clear-selection').disabled=false;renderSelectionOverlay();renderSelectionSummary();loadFlamegraph().catch(showError);
}
function clearSelection(){state.selection=null;state.selectedEvent=null;$('zoom-selection').disabled=true;$('clear-selection').disabled=true;renderSelectionOverlay();renderSelectionSummary();loadFlamegraph().catch(showError)}
function trackLabel(value){
  const text=String(value);if(text.startsWith('cpu:'))return `CPU ${text.slice(4)}`;if(text.startsWith('pid:'))return `PID ${text.slice(4)}`;return `${state.trackMode.toUpperCase()} ${text}`;
}
function trackFieldValue(value){const match=/^(cpu|pid):(-?\d+)$/.exec(String(value));return match?{field:match[1],value:Number(match[2])}:{field:state.trackMode,value:Number(value)}}
function renderSelectionSummary(){
  const target=$('selection-summary');
  if(state.selectedEvent){const e=state.selectedEvent;target.textContent=`${e.name||'unnamed'} · ${e.category} · ${trackLabel(e.track)} · ${e.start.toFixed(6)}s · ${(Math.max(0,e.end-e.start)*1000).toFixed(3)} ms`;return}
  if(state.selection){target.textContent=`Selected ${state.selection[0].toFixed(6)}s – ${state.selection[1].toFixed(6)}s · ${((state.selection[1]-state.selection[0])*1000).toFixed(3)} ms`;return}
  target.textContent='Select an event or drag across tracks to inspect a region.';
}
function renderSelectionOverlay(){
  const selection=$('time-selection');if(!state.selection||!state.range){selection.style.display='none';return}
  const [start,end]=state.range,[a,b]=state.selection,plotStart=TIMELINE_LABEL_WIDTH,canvasWidth=$('timeline').getBoundingClientRect().width,plotWidth=Math.max(1,canvasWidth-plotStart);
  if(b<start||a>end){selection.style.display='none';return}
  const left=plotStart+Math.max(0,(a-start)/(end-start))*plotWidth,right=plotStart+Math.min(1,(b-start)/(end-start))*plotWidth;
  selection.style.display='block';selection.style.left=`${left}px`;selection.style.width=`${Math.max(1,right-left)}px`;
}
function hitAt(x,y){for(let i=state.hitRegions.length-1;i>=0;i--){const hit=state.hitRegions[i];if(x>=hit.x&&x<=hit.x+hit.w&&y>=hit.y&&y<=hit.y+hit.h)return hit}return null}
function hideTimelineTooltip(){$('timeline-tooltip').hidden=true}
function showTimelineTooltip(event){
  const hit=hitAt(event.offsetX,event.offsetY),tooltip=$('timeline-tooltip');if(!hit){hideTimelineTooltip();return}
  const duration=Math.max(0,hit.end-hit.start),details=[`<strong>${escapeHtml(hit.name||'(unnamed)')}</strong>`,`<span>${escapeHtml(hit.category)} · ${escapeHtml(trackLabel(hit.track))}</span>`,`<span>${hit.start.toFixed(9)}s · ${(duration*1e6).toFixed(2)} µs</span>`];
  tooltip.innerHTML=details.join('');tooltip.hidden=false;tooltip.style.left=`${Math.min(event.offsetX+14,Math.max(8,$('timeline').clientWidth-260))}px`;tooltip.style.top=`${Math.max(4,event.offsetY-8)}px`;
}
async function selectTimelineHit(hit,time){
  if(hit.id!==undefined){setSelection(hit.start,hit.end,hit);return}
  const selected=trackFieldValue(hit.track),epsilon=Math.max((state.range[1]-state.range[0])/Math.max(1,$('timeline').getBoundingClientRect().width-TIMELINE_LABEL_WIDTH),1e-9),result=await query(`SELECT id,ts,dur,cpu,pid,event,name,category FROM events ${where(`${selected.field}=${selected.value} AND ts<${time+epsilon} AND ts_end>${time-epsilon}`)} ORDER BY ABS(ts-${time}),dur DESC LIMIT 1`,1);
  if(!result.rows.length){setSelection(hit.start,hit.end,hit);return}const [id,start,dur,cpu,pid,event,name,category]=result.rows[0],track=state.trackMode==='cpu_pid'?`${selected.field}:${selected.field==='cpu'?cpu:pid}`:(state.trackMode==='cpu'?cpu:pid);setSelection(start,start+Math.max(dur,1e-9),{id,start,end:start+dur,track,event,name,category});
}
function installTimelineNavigation(canvas,width){
  const plotStart=TIMELINE_LABEL_WIDTH,plotWidth=Math.max(1,width-plotStart),selection=$('time-selection');let drag=null;
  const plotX=event=>Math.max(0,Math.min(plotWidth,event.offsetX-plotStart));
  const timeAt=x=>state.range[0]+(state.range[1]-state.range[0])*x/plotWidth;
  canvas.onpointerdown=event=>{
    if(event.button!==0||event.offsetX<plotStart)return;hideTimelineTooltip();drag={x:plotX(event),last:plotX(event),range:[...state.range],pan:event.shiftKey,moved:false,pointer:event.pointerId};canvas.setPointerCapture(event.pointerId);event.preventDefault();
    if(!drag.pan){selection.style.display='block';selection.style.left=`${plotStart+drag.x}px`;selection.style.width='0px'}
  };
  canvas.onpointermove=event=>{
    if(!drag){showTimelineTooltip(event);return}hideTimelineTooltip();const current=plotX(event);drag.moved=drag.moved||Math.abs(current-drag.x)>=3;
    if(drag.pan){const delta=(drag.x-current)*(drag.range[1]-drag.range[0])/plotWidth;state.range=boundedRange(drag.range[0]+delta,drag.range[1]+delta);updateOverviewViewport()}
    else{const left=Math.min(drag.x,current);selection.style.display='block';selection.style.left=`${plotStart+left}px`;selection.style.width=`${Math.abs(current-drag.x)}px`}
    drag.last=current;
  };
  const cancel=()=>{drag=null;renderSelectionOverlay()};canvas.onpointercancel=cancel;
  canvas.onpointerup=event=>{
    if(!drag)return;const finished=drag;drag=null;
    if(finished.pan){refresh();return}
    if(finished.moved)setSelection(timeAt(Math.min(finished.x,finished.last)),timeAt(Math.max(finished.x,finished.last)));
    else{const hit=hitAt(event.offsetX,event.offsetY);if(hit)selectTimelineHit(hit,timeAt(finished.last)).catch(showError);else clearSelection()}
  };
  canvas.ondblclick=event=>{event.preventDefault();setRange(...state.full)};
  canvas.onpointerleave=()=>{if(!drag)hideTimelineTooltip()};
  canvas.onwheel=event=>{
    if(event.ctrlKey||event.metaKey){event.preventDefault();const fraction=plotX(event)/plotWidth,[a,b]=state.range,anchor=a+(b-a)*fraction;zoomRange(Math.exp(event.deltaY*.002),anchor)}
    else if(event.shiftKey||Math.abs(event.deltaX)>Math.abs(event.deltaY)){event.preventDefault();const delta=(event.deltaX||event.deltaY)*(state.range[1]-state.range[0])/plotWidth;setRange(state.range[0]+delta,state.range[1]+delta)}
  };
}
function exactTimelineSql(start,end,bucket,bucketCount,trackScope,track='cpu'){return `WITH RECURSIVE scoped_events(${track},event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT ${track},event,name,category,ipc,ts,ts_end,
           MAX(0,CAST((ts-${start})/${bucket} AS INTEGER)),
           MIN(${bucketCount-1},CAST((ts_end-${start})/${bucket} AS INTEGER))
      FROM events ${where(`ts < ${end} AND ts_end > ${start} AND ${trackScope}`)}
  ), expanded(${track},event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT * FROM scoped_events
    UNION ALL SELECT ${track},event,name,category,ipc,ts,ts_end,bucket+1,last_bucket FROM expanded WHERE bucket<last_bucket
  ), scored AS (
    SELECT bucket,${track},event,name,category,ipc,
           SUM(MIN(ts_end,${start}+(bucket+1)*${bucket})-MAX(ts,${start}+bucket*${bucket})) weight,COUNT(*) count
      FROM expanded WHERE ts<${start}+(bucket+1)*${bucket} AND ts_end>${start}+bucket*${bucket}
     GROUP BY bucket,${track},event,name,category,ipc),
  ranked AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket,${track} ORDER BY weight DESC,count DESC,event,name,category,ipc) rank FROM scored)
  SELECT bucket,${track},event,name,category,ipc,weight,count FROM ranked WHERE rank=1 ORDER BY ${track},bucket`}
function mipmapTimelineSql(start,end,bucket,bucketCount,cpuScope,table){
  const maxDuration=state.mipmapWidth*state.mipmapMaxBins,mipmapScope=[cpuScope,filterSql(),visibilitySql()].filter(Boolean).join(' AND ');
  return `WITH RECURSIVE mapped(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
    SELECT MAX(0,CAST((bucket_start-${start})/${bucket} AS INTEGER)),MIN(${bucketCount-1},CAST((bucket_end-${start})/${bucket} AS INTEGER)),cpu,event,name,category,ipc,bucket_start,bucket_end,weight,count
      FROM ${table} WHERE bucket_start<${end} AND bucket_end>${start} AND ${mipmapScope}
  ), map_expanded(bucket,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count) AS (
    SELECT * FROM mapped UNION ALL SELECT bucket+1,last_bucket,cpu,event,name,category,ipc,source_start,source_end,weight,count FROM map_expanded WHERE bucket<last_bucket
  ), long_events(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT cpu,event,name,category,ipc,ts,ts_end,MAX(0,CAST((ts-${start})/${bucket} AS INTEGER)),MIN(${bucketCount-1},CAST((ts_end-${start})/${bucket} AS INTEGER))
      FROM events WHERE (dur=0 OR dur>${maxDuration}) AND ${mipmapScope} AND ((dur=0 AND ts>=${start} AND ts<${end}) OR (dur>0 AND ts<${end} AND ts_end>${start}))
  ), long_expanded(cpu,event,name,category,ipc,ts,ts_end,bucket,last_bucket) AS (
    SELECT * FROM long_events UNION ALL SELECT cpu,event,name,category,ipc,ts,ts_end,bucket+1,last_bucket FROM long_expanded WHERE bucket<last_bucket
  ), combined AS (
    SELECT bucket,cpu,event,name,category,ipc,weight*MAX(0,MIN(source_end,${end},${start}+(bucket+1)*${bucket})-MAX(source_start,${start},${start}+bucket*${bucket}))/(source_end-source_start) weight,count FROM map_expanded
    UNION ALL SELECT bucket,cpu,event,name,category,ipc,MAX(0,MIN(ts_end,${end},${start}+(bucket+1)*${bucket})-MAX(ts,${start},${start}+bucket*${bucket})) weight,1 count FROM long_expanded
  ), scored AS (SELECT bucket,cpu,event,name,category,ipc,SUM(weight) weight,SUM(count) count FROM combined GROUP BY bucket,cpu,event,name,category,ipc),
  ranked AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY bucket,cpu ORDER BY weight DESC,count DESC,event,name,category,ipc) rank FROM scored)
  SELECT bucket,cpu,event,name,category,ipc,weight,count FROM ranked WHERE rank=1 ORDER BY cpu,bucket`;
}
function searchMatch(row){
  if(!state.search)return false;const needle=state.search.toLocaleLowerCase(),haystack=[row.name,row.category,row.track,row.event].join(' ').toLocaleLowerCase(),match=haystack.includes(needle);return state.searchInvert?!match:match;
}
function rawTimelineSql(start,end,trackScope,track){return `SELECT id,ts,dur,ts_end,${track},cpu,pid,event,name,category,arg0,retval,ipc,rpc FROM events ${where(`ts < ${end} AND ts_end > ${start} AND ${trackScope}`)} ORDER BY ${track},ts,dur DESC`}
function timelineLane(category){if(category==='user'||category==='agent')return 0;if(category==='syscall')return 1;if(category==='kernel'||category==='scheduler')return 2;return 3}
function prepareTimelineCanvas(canvas,width,height,ratio){canvas.style.height=`${height}px`;canvas.width=Math.max(300,Math.round(width*ratio));canvas.height=Math.round(height*ratio);const ctx=canvas.getContext('2d');ctx.scale(ratio,ratio);ctx.fillStyle='#080b11';ctx.fillRect(0,0,width,height);ctx.font='10px ui-monospace,monospace';return ctx}
function drawTimelineGrid(ctx,width,height,start,end){const plotWidth=width-TIMELINE_LABEL_WIDTH;ctx.save();ctx.strokeStyle='#202a39';ctx.fillStyle='#6f7d93';ctx.textAlign='center';for(let i=0;i<=10;i++){const x=TIMELINE_LABEL_WIDTH+plotWidth*i/10;ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,height);ctx.stroke();if(i>0&&i<10)ctx.fillText(`${((start+(end-start)*i/10)*1000).toFixed(3)}ms`,x,10)}ctx.restore()}
function makeTrackLayout(tracks,viewportHeight,regularHeight,globalHeight){const base=tracks.map(value=>state.trackMode==='cpu'&&Number(value)<0?globalHeight:regularHeight),minimum=base.reduce((sum,value)=>sum+value,0),height=Math.max(64,viewportHeight,minimum),flex=base.map((_,index)=>index).filter(index=>base[index]===regularHeight),targets=flex.length?flex:base.map((_,index)=>index),bonus=(height-minimum)/Math.max(1,targets.length),layout=new Map();let y=0;tracks.forEach((value,index)=>{const rowH=base[index]+(targets.includes(index)?bonus:0),global=state.trackMode==='cpu'&&Number(value)<0;layout.set(value,{y,rowH,global});y+=rowH});return {height,layout}}
const kutraceLight=['#f7b6d2','#a8e6cf','#ffd3a5','#b5d8ff','#d5b3ff','#ffe58a','#b8f2e6','#ffcab1','#c7ceea','#c9f0c1','#f6c1c7','#a0d8ef','#e2c2ff','#f9e2ae','#bde0fe','#cdeac0','#ffc8dd'];
const kutraceDark=['#b00060','#008060','#d06000','#0050b0','#7030a0','#a07800','#007c78','#b04020','#3f51a3','#348c31','#a52a3a','#166088','#663399','#8a5a00','#1d5d9b'];
function kutraceColors(event){const value=Math.abs(Number(event)||0);return [kutraceLight[(value*4)%kutraceLight.length],kutraceDark[(value*7+6)%kutraceDark.length]]}
function kutraceEventKind(event,category){const value=Number(event);if(value===-3)return'arc';if(value===65536)return'idle';if(value===131072)return'c-exit';if(value>=522&&value<=525)return'mark';if(value===521||value===540)return'frequency';if((value&~1)===640)return'sample';if((value&~1)===642)return'lock';if((value&0xfffe0)===768)return'wait';if(category==='user')return'user';if(category==='agent')return'agent';if(category==='syscall')return'syscall';if(category==='kernel'||category==='scheduler')return'kernel';if(category==='wakeup')return'wakeup';return'special'}
function drawKutraceSpan(ctx,x,w,center,bandH,event,name,category,arg0,ipc,matched,selected){
  const kind=kutraceEventKind(event,category),[light,dark]=kutraceColors(event);ctx.save();ctx.globalAlpha=state.search&&!matched?.16:.96;
  if(kind==='idle'){ctx.strokeStyle='#111';ctx.lineWidth=Math.max(1,bandH*.06);ctx.setLineDash(Number(arg0)===1?[7,3]:[]);ctx.beginPath();ctx.moveTo(x,center);ctx.lineTo(x+w,center);ctx.stroke()}
  else if(kind==='user'||kind==='agent'){const top=kind==='agent'?'#d783ff':light,bottom=kind==='agent'?'#713b92':dark;ctx.fillStyle=top;ctx.fillRect(x,center-bandH*.19,w,bandH*.23);ctx.fillStyle=bottom;ctx.fillRect(x,center+bandH*.04,w,bandH*.15)}
  else if(kind==='kernel'||kind==='syscall'){const outer=kind==='syscall'?'#808080':((Number(event)&0xfff00)===0x500?'#000':'#55aa55');ctx.fillStyle=outer;ctx.fillRect(x,center-bandH*.5,w,bandH);ctx.fillStyle=kind==='syscall'?'#d8d8d8':light;ctx.fillRect(x,center-bandH*.465,w,bandH*.93);ctx.fillStyle=dark;ctx.fillRect(x,center-bandH*.265,w,bandH*.53);ctx.fillStyle=light;ctx.fillRect(x,center-bandH*.1,w,bandH*.2)}
  else if(kind==='wait'){ctx.strokeStyle=dark;ctx.lineWidth=Math.max(2,bandH*.08);ctx.setLineDash([2,3]);ctx.beginPath();ctx.moveTo(x,center-bandH*.3);ctx.lineTo(x+w,center-bandH*.3);ctx.stroke()}
  else if(kind==='frequency'&&state.overlays.frequency){ctx.fillStyle='rgba(80,180,90,.28)';ctx.fillRect(x,center-bandH*.58,w,bandH*.16);if(w>28){ctx.fillStyle='#174d20';ctx.font='9px ui-monospace,monospace';ctx.fillText(`${arg0}MHz`,x+2,center-bandH*.43)}}
  else if(kind==='sample'&&state.overlays.samples){ctx.strokeStyle=dark;ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(x,center-bandH*.7);ctx.lineTo(x,center-bandH*.48);ctx.stroke()}
  else if(kind==='lock'&&state.overlays.locks){ctx.strokeStyle=dark;ctx.lineWidth=2;ctx.setLineDash(Number(event)===643?[4,3]:[]);ctx.beginPath();ctx.moveTo(x,center-bandH*.62);ctx.lineTo(x+w,center-bandH*.62);ctx.stroke()}
  else if(kind==='mark'&&state.overlays.marks){const mark=Number(event)-521;ctx.fillStyle=['#005bbb','#d02020','#00843d','#8b3fb0'][mark-1]||dark;ctx.beginPath();ctx.moveTo(x,center-bandH*.72);ctx.lineTo(x-4,center-bandH*.48);ctx.lineTo(x+4,center-bandH*.48);ctx.closePath();ctx.fill();ctx.font='bold 9px ui-monospace,monospace';ctx.fillText(name||String(mark),x+5,center-bandH*.55)}
  else if(kind==='wakeup'){ctx.fillStyle='#d02020';ctx.beginPath();ctx.arc(x,center-bandH*.55,3,0,Math.PI*2);ctx.fill()}
  else if(kind!=='arc'){ctx.fillStyle=dark;ctx.fillRect(x,center-bandH*.15,Math.max(2,w),bandH*.3)}
  ctx.globalAlpha=1;ctx.setLineDash([]);if(ipc&&state.overlays.ipc){ctx.fillStyle='#111';ctx.beginPath();ctx.moveTo(x+w/2,center-bandH*.48);ctx.lineTo(x+w/2-3,center-bandH*.36);ctx.lineTo(x+w/2+3,center-bandH*.36);ctx.closePath();ctx.fill()}
  if(selected){ctx.strokeStyle='#d00000';ctx.lineWidth=2;ctx.strokeRect(x-1,center-bandH*.55,w+2,bandH*1.1)}ctx.restore();
}
function drawKutraceGrid(ctx,width,height,start,end){const plotWidth=width-TIMELINE_LABEL_WIDTH;ctx.save();ctx.strokeStyle='#e1e4e8';ctx.fillStyle='#374151';ctx.textAlign='center';ctx.font='10px ui-monospace,monospace';for(let i=0;i<=10;i++){const x=TIMELINE_LABEL_WIDTH+plotWidth*i/10;ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,height);ctx.stroke();if(i>0&&i<10)ctx.fillText(`${((start+(end-start)*i/10)*1000).toFixed(3)}`,x,12)}ctx.restore()}
function drawKutraceArc(ctx,x0,y0,x1,y1){const lift=Math.max(18,Math.min(58,Math.abs(x1-x0)*.22)),controlY=Math.min(y0,y1)-lift;ctx.save();ctx.strokeStyle='rgba(190,0,0,.82)';ctx.fillStyle='#be0000';ctx.lineWidth=1.4;ctx.beginPath();ctx.moveTo(x0,y0);ctx.quadraticCurveTo((x0+x1)/2,controlY,x1,y1);ctx.stroke();const angle=Math.atan2(y1-controlY,x1-(x0+x1)/2);ctx.beginPath();ctx.moveTo(x1,y1);ctx.lineTo(x1-7*Math.cos(angle-.45),y1-7*Math.sin(angle-.45));ctx.lineTo(x1-7*Math.cos(angle+.45),y1-7*Math.sin(angle+.45));ctx.closePath();ctx.fill();ctx.restore()}
function drawKutraceTimeline(canvas,width,ratio,start,end,tracks,result){
  const viewportHeight=canvas.closest('.timeline-scroll').clientHeight,{height,layout}=makeTrackLayout(tracks,viewportHeight,78,36),plotWidth=Math.max(1,width-TIMELINE_LABEL_WIDTH),ctx=prepareTimelineCanvas(canvas,width,height,ratio);ctx.fillStyle='#fff';ctx.fillRect(0,0,width,height);state.hitRegions=[];
  for(const [index,value] of tracks.entries()){const row=layout.get(value),groupStart=state.trackMode==='cpu_pid'&&(index===0||String(value).split(':')[0]!==String(tracks[index-1]).split(':')[0]);ctx.fillStyle=index%2?'#fbfbfb':'#fff';ctx.fillRect(0,row.y,width,row.rowH);ctx.fillStyle='#f6f7f8';ctx.fillRect(0,row.y,TIMELINE_LABEL_WIDTH,row.rowH);ctx.strokeStyle=groupStart?'#0836c7':'#d5d9df';ctx.lineWidth=groupStart?2:1;ctx.beginPath();ctx.moveTo(0,row.y+.5);ctx.lineTo(width,row.y+.5);ctx.stroke();ctx.strokeStyle='#d5d9df';ctx.lineWidth=1;ctx.beginPath();ctx.moveTo(0,row.y+row.rowH-.5);ctx.lineTo(width,row.y+row.rowH-.5);ctx.stroke();ctx.fillStyle='#0836c7';ctx.font='bold 12px ui-monospace,monospace';ctx.fillText(trackLabel(value),8,row.y+row.rowH/2+4)}drawKutraceGrid(ctx,width,height,start,end);
  const arcs=[],ordered=result.rows.slice().sort((a,b)=>Number(b[2])-Number(a[2]));ctx.save();ctx.beginPath();ctx.rect(TIMELINE_LABEL_WIDTH,0,plotWidth,height);ctx.clip();
  for(const [id,ts,dur,tsEnd,value,cpu,pid,event,name,category,arg0,retval,ipc,rpc] of ordered){const row=layout.get(value);if(!row)continue;const eventStart=Math.max(start,Number(ts)),eventEnd=Math.min(end,Math.max(Number(tsEnd),Number(ts)+(end-start)/plotWidth)),x=TIMELINE_LABEL_WIDTH+(eventStart-start)*plotWidth/(end-start),w=Math.max(Number(dur)>0?1.25:2.5,(eventEnd-eventStart)*plotWidth/(end-start)),center=row.y+row.rowH*.55,bandH=Math.max(24,Math.min(96,row.rowH*.62)),hit={id,x,y:center-bandH*.6,w,h:bandH*1.2,track:value,cpu,pid,event,name,category,arg0,retval,ipc,rpc,start:Number(ts),end:Math.max(Number(tsEnd),Number(ts)+1e-9)},matched=searchMatch(hit);if(Number(event)===-3){arcs.push({x0:x,x1:x+w,y0:center,target:Number(arg0)});continue}drawKutraceSpan(ctx,x,w,center,bandH,event,name,category,arg0,ipc,matched,state.selectedEvent&&state.selectedEvent.id===id);if(w>38&&name&&!['mark','frequency'].includes(kutraceEventKind(event,category))){ctx.save();ctx.beginPath();ctx.rect(x+2,center-bandH*.48,w-4,bandH*.96);ctx.clip();ctx.fillStyle='#111';ctx.font='10px ui-sans-serif,system-ui';ctx.fillText(name,x+4,center-2);ctx.restore()}state.hitRegions.push(hit)}
  if(state.overlays.arcs&&state.trackMode!=='pid')for(const arc of arcs){const target=layout.get(state.trackMode==='cpu_pid'?`cpu:${arc.target}`:arc.target)||layout.get(tracks[0]);if(target)drawKutraceArc(ctx,arc.x0,arc.y0,arc.x1,target.y+target.rowH*.55)}ctx.restore();canvas.dataset.source='events';canvas.dataset.mipmapLevel='none';canvas.dataset.detail='true';canvas.dataset.renderer='kutrace';$('timeline-mode').textContent=`Native KUtrace · ${result.rows.length.toLocaleString()} exact rows`;return height;
}
function drawKutraceSummary(canvas,width,ratio,start,end,tracks,result,bucket,pixelPerBucket,source,level){
  const viewportHeight=canvas.closest('.timeline-scroll').clientHeight,{height,layout}=makeTrackLayout(tracks,viewportHeight,44,28),ctx=prepareTimelineCanvas(canvas,width,height,ratio);ctx.fillStyle='#fff';ctx.fillRect(0,0,width,height);state.hitRegions=[];for(const value of tracks){const row=layout.get(value);ctx.fillStyle='#f6f7f8';ctx.fillRect(0,row.y,TIMELINE_LABEL_WIDTH,row.rowH);ctx.strokeStyle='#d5d9df';ctx.beginPath();ctx.moveTo(0,row.y+row.rowH-.5);ctx.lineTo(width,row.y+row.rowH-.5);ctx.stroke();ctx.fillStyle='#0836c7';ctx.font='bold 11px ui-monospace,monospace';ctx.fillText(trackLabel(value),8,row.y+row.rowH/2+4)}drawKutraceGrid(ctx,width,height,start,end);for(const [bin,value,event,name,category,ipc] of result.rows){const row=layout.get(value);if(!row)continue;const x=TIMELINE_LABEL_WIDTH+bin*pixelPerBucket,w=Math.max(1,Math.ceil(pixelPerBucket)),center=row.y+row.rowH*.55,bandH=Math.max(16,row.rowH*.62),hit={x,y:center-bandH*.6,w,h:bandH*1.2,track:value,event,name,category,start:start+bin*bucket,end:Math.min(end,start+(bin+1)*bucket)};drawKutraceSpan(ctx,x,w,center,bandH,event,name,category,0,ipc,searchMatch(hit),false);state.hitRegions.push(hit)}canvas.dataset.source=source;canvas.dataset.mipmapLevel=level;canvas.dataset.detail='false';canvas.dataset.renderer='kutrace';$('timeline-mode').textContent='Native KUtrace density · zoom for exact events';return height;
}
function drawDetailedTimeline(canvas,width,ratio,start,end,tracks,result){
  const viewportHeight=canvas.closest('.timeline-scroll').clientHeight,{height,layout}=makeTrackLayout(tracks,viewportHeight,94,42),plotWidth=Math.max(1,width-TIMELINE_LABEL_WIDTH),ctx=prepareTimelineCanvas(canvas,width,height,ratio),palette=state.overlays.colorblind?colorblindColors:colors;state.hitRegions=[];
  for(const [index,value] of tracks.entries()){const row=layout.get(value),{y,rowH,global}=row,laneTop=19,laneH=global?rowH-laneTop-5:Math.max(17,(rowH-laneTop-5)/laneLabels.length);ctx.fillStyle=index%2?'#0b111a':'#0d141f';ctx.fillRect(0,y,width,rowH);ctx.fillStyle='#111a27';ctx.fillRect(0,y,TIMELINE_LABEL_WIDTH,rowH);ctx.strokeStyle='#344158';ctx.beginPath();ctx.moveTo(0,y+rowH-.5);ctx.lineTo(width,y+rowH-.5);ctx.stroke();ctx.fillStyle='#dbe5f5';ctx.font='bold 11px ui-monospace,monospace';ctx.fillText(global?'GLOBAL':trackLabel(value),8,y+14);ctx.font='9px ui-monospace,monospace';ctx.fillStyle='#718097';(global?['markers / I/O']:laneLabels).forEach((label,lane)=>ctx.fillText(label,8,y+laneTop+lane*laneH+11))}
  drawTimelineGrid(ctx,width,height,start,end);
  ctx.save();ctx.beginPath();ctx.rect(TIMELINE_LABEL_WIDTH,0,plotWidth,height);ctx.clip();
  for(const [id,ts,dur,tsEnd,value,cpu,pid,event,name,category,arg0,retval,ipc,rpc] of result.rows){const row=layout.get(value);if(!row)continue;const laneTop=19,laneH=row.global?row.rowH-laneTop-5:Math.max(17,(row.rowH-laneTop-5)/laneLabels.length),lane=row.global?0:timelineLane(category),y=row.y+laneTop+lane*laneH,eventStart=Math.max(start,Number(ts)),eventEnd=Math.min(end,Math.max(Number(tsEnd),Number(ts)+(end-start)/plotWidth)),x=TIMELINE_LABEL_WIDTH+(eventStart-start)*plotWidth/(end-start),w=Math.max(dur>0?1.5:2.5,(eventEnd-eventStart)*plotWidth/(end-start)),h=laneH-2,hit={id,x,y,w,h,track:value,cpu,pid,event,name,category,arg0,retval,ipc,rpc,start:Number(ts),end:Math.max(Number(tsEnd),Number(ts)+1e-9)},matched=searchMatch(hit);ctx.fillStyle=palette[category]||palette.special;ctx.globalAlpha=state.search&&!matched?.18:.92;ctx.fillRect(x,y,w,h);ctx.globalAlpha=1;if(state.search&&matched){ctx.strokeStyle='#fff';ctx.lineWidth=1.5;ctx.strokeRect(x+.5,y+.5,Math.max(1,w-1),h-1)}if(state.selectedEvent&&state.selectedEvent.id===id){ctx.strokeStyle='#fff';ctx.lineWidth=2;ctx.strokeRect(x-1,y-1,w+2,h+2)}if(w>34&&name){ctx.save();ctx.beginPath();ctx.rect(x+2,y,w-4,h);ctx.clip();ctx.fillStyle='#071019';ctx.font='10px ui-sans-serif,system-ui';ctx.fillText(name,x+4,y+11);ctx.restore()}if(ipc&&state.overlays.ipc){ctx.fillStyle='#fff';ctx.fillRect(x,y,Math.max(1,w),2)}state.hitRegions.push(hit)}
  ctx.restore();canvas.dataset.source='events';canvas.dataset.mipmapLevel='none';canvas.dataset.detail='true';canvas.dataset.renderer='lanes';$('timeline-mode').textContent=`Event lanes · ${result.rows.length.toLocaleString()} exact events`;return height;
}
function drawSummaryTimeline(canvas,width,ratio,start,end,tracks,result,bucket,pixelPerBucket,source,level){
  const viewportHeight=canvas.closest('.timeline-scroll').clientHeight,{height,layout}=makeTrackLayout(tracks,viewportHeight,32,24),ctx=prepareTimelineCanvas(canvas,width,height,ratio),palette=state.overlays.colorblind?colorblindColors:colors;state.hitRegions=[];
  for(const [index,value] of tracks.entries()){const {y,rowH,global}=layout.get(value);ctx.fillStyle=index%2?'#0b111a':'#0d141f';ctx.fillRect(0,y,width,rowH);ctx.fillStyle='#111a27';ctx.fillRect(0,y,TIMELINE_LABEL_WIDTH,rowH);ctx.fillStyle='#dbe5f5';ctx.font='bold 11px ui-monospace,monospace';ctx.fillText(global?'GLOBAL':trackLabel(value),8,y+Math.min(19,rowH/2+4));ctx.strokeStyle='#344158';ctx.beginPath();ctx.moveTo(0,y+rowH-.5);ctx.lineTo(width,y+rowH-.5);ctx.stroke()}
  drawTimelineGrid(ctx,width,height,start,end);
  for(const [bin,value,event,name,category,ipc] of result.rows){const x=TIMELINE_LABEL_WIDTH+bin*pixelPerBucket,row=layout.get(value);if(!row)continue;const h=Math.max(2,Math.min(28,row.rowH-6)),y=row.y+(row.rowH-h)/2,w=Math.ceil(pixelPerBucket),hit={x,y,w,h,track:value,event,name,category,start:start+bin*bucket,end:Math.min(end,start+(bin+1)*bucket)};ctx.fillStyle=palette[category]||palette.special;ctx.globalAlpha=state.search&&!searchMatch(hit)?.2:1;ctx.fillRect(x,y,w,h);ctx.globalAlpha=1;if(state.search&&searchMatch(hit)){ctx.strokeStyle='#fff';ctx.strokeRect(x+.5,y+.5,Math.max(1,w-1),h-1)}if(ipc&&state.overlays.ipc){ctx.fillStyle='#fff';ctx.fillRect(x,y,Math.max(1,w),2)}state.hitRegions.push(hit)}
  canvas.dataset.source=source;canvas.dataset.mipmapLevel=level;canvas.dataset.detail='false';canvas.dataset.renderer='lanes';$('timeline-mode').textContent='Event-lane density · zoom in for exact events';return height;
}
async function drawTimeline(signal){
  if(!state.range)return;const canvas=$('timeline'),ratio=devicePixelRatio||1,width=canvas.getBoundingClientRect().width||800,plotWidth=Math.max(1,width-TIMELINE_LABEL_WIDTH),[start,end]=state.range,track=state.trackMode;
  canvas.dataset.ready='false';canvas.dataset.trackMode=track;updateRangeChrome();
  if(track==='cpu_pid'){
    canvas.dataset.trackGroups='cpu,pid';
    const scoped=where(`ts < ${end} AND ts_end > ${start}`),[cpuResult,pidResult]=await Promise.all([
      state.groups.cpu?query(`SELECT DISTINCT cpu FROM events ${scoped} ORDER BY cpu LIMIT 10000`,10000,signal):Promise.resolve({rows:[]}),
      state.groups.process?query(`SELECT DISTINCT pid FROM events ${scoped} ORDER BY pid LIMIT 10000`,10000,signal):Promise.resolve({rows:[]})
    ]),allTracks=[...cpuResult.rows.map(row=>`cpu:${row[0]}`).filter(value=>state.timelineRenderer!=='kutrace'||Number(value.slice(4))>=0),...pidResult.rows.map(row=>`pid:${row[0]}`)],pageCount=Math.max(1,Math.ceil(allTracks.length/state.cpuPageSize));
    state.cpuPage=Math.min(state.cpuPage,pageCount-1);const first=state.cpuPage*state.cpuPageSize,tracks=allTracks.slice(first,first+state.cpuPageSize),cpuValues=tracks.filter(value=>value.startsWith('cpu:')).map(value=>Number(value.slice(4))),pidValues=tracks.filter(value=>value.startsWith('pid:')).map(value=>Number(value.slice(4))),scopeParts=[];if(cpuValues.length)scopeParts.push(`cpu IN (${cpuValues.join(',')})`);if(pidValues.length)scopeParts.push(`pid IN (${pidValues.join(',')})`);const trackScope=scopeParts.length?`(${scopeParts.join(' OR ')})`:'0';
    $('cpu-controls').hidden=pageCount===1;$('previous-cpus').disabled=state.cpuPage===0;$('next-cpus').disabled=state.cpuPage>=pageCount-1;$('cpu-window').textContent=allTracks.length?`Rows ${first+1}–${first+tracks.length} of ${allTracks.length}`:'No CPU/PID tracks';
    const raw=await query(rawTimelineSql(start,end,trackScope,'cpu'),DETAIL_EVENT_LIMIT,signal),selected=new Set(tracks),rows=[];for(const row of raw.rows){const cpuKey=`cpu:${row[5]}`,pidKey=`pid:${row[6]}`;if(selected.has(cpuKey)){const copy=row.slice();copy[4]=cpuKey;rows.push(copy)}if(selected.has(pidKey)){const copy=row.slice();copy[4]=pidKey;rows.push(copy)}}
    let height;if(!raw.truncated){const result={...raw,rows};height=state.timelineRenderer==='kutrace'?drawKutraceTimeline(canvas,width,ratio,start,end,tracks,result):drawDetailedTimeline(canvas,width,ratio,start,end,tracks,result)}
    if(height===undefined){const bucketCount=Math.max(1,Math.min(Math.floor(plotWidth),Math.floor(8000/Math.max(1,tracks.length)))),bucket=(end-start)/bucketCount,pixelPerBucket=plotWidth/bucketCount,requests=[];if(cpuValues.length)requests.push(query(exactTimelineSql(start,end,bucket,bucketCount,`cpu IN (${cpuValues.join(',')})`,'cpu'),10000,signal).then(result=>result.rows.map(row=>[row[0],`cpu:${row[1]}`,...row.slice(2)])));if(pidValues.length)requests.push(query(exactTimelineSql(start,end,bucket,bucketCount,`pid IN (${pidValues.join(',')})`,'pid'),10000,signal).then(result=>result.rows.map(row=>[row[0],`pid:${row[1]}`,...row.slice(2)])));const summaryRows=(await Promise.all(requests)).flat();height=state.timelineRenderer==='kutrace'?drawKutraceSummary(canvas,width,ratio,start,end,tracks,{rows:summaryRows},bucket,pixelPerBucket,'events','none'):drawSummaryTimeline(canvas,width,ratio,start,end,tracks,{rows:summaryRows},bucket,pixelPerBucket,'events','none')}
    installTimelineNavigation(canvas,width);renderSelectionOverlay();canvas.dataset.preview='false';canvas.dataset.ready='true';return;
  }
  canvas.dataset.trackGroups=track;
  const scoped=where(`ts < ${end} AND ts_end > ${start}`),trackResult=await query(`SELECT DISTINCT ${track} FROM events ${scoped} ORDER BY ${track} LIMIT 10000`,10000,signal),allTracks=trackResult.rows.map(row=>row[0]).filter(value=>state.timelineRenderer!=='kutrace'||track!=='cpu'||Number(value)>=0),pageCount=Math.max(1,Math.ceil(allTracks.length/state.cpuPageSize));
  state.cpuPage=Math.min(state.cpuPage,pageCount-1);const first=state.cpuPage*state.cpuPageSize,tracks=allTracks.slice(first,first+state.cpuPageSize),trackScope=tracks.length?`${track} IN (${tracks.join(',')})`:'0';
  $('cpu-controls').hidden=pageCount===1;$('previous-cpus').disabled=state.cpuPage===0;$('next-cpus').disabled=state.cpuPage>=pageCount-1;$('cpu-window').textContent=allTracks.length?`${track==='cpu'?'CPUs':'PIDs'} ${first+1}–${first+tracks.length} of ${allTracks.length}`:`No ${track.toUpperCase()} tracks`;
  const bucketCount=Math.max(1,Math.min(Math.floor(plotWidth),Math.floor(8000/Math.max(1,tracks.length)))),bucket=(end-start)/bucketCount,pixelPerBucket=plotWidth/bucketCount;
  const mipmapCompatible=track==='cpu'&&state.filters.every(filter=>['category','cpu','event'].includes(filter.field)),useMipmap=mipmapCompatible&&state.mipmapWidth>0&&bucket>=state.mipmapWidth*4,useCoarse=useMipmap&&state.mipmapCoarseWidth>0&&bucket>=state.mipmapCoarseWidth*4,table=useCoarse?'timeline_mipmap_coarse':'timeline_mipmap',sql=useMipmap?mipmapTimelineSql(start,end,bucket,bucketCount,trackScope,table):exactTimelineSql(start,end,bucket,bucketCount,trackScope,track);
  let height;if(!useMipmap){const raw=await query(rawTimelineSql(start,end,trackScope,track),DETAIL_EVENT_LIMIT,signal);if(!raw.truncated)height=state.timelineRenderer==='kutrace'?drawKutraceTimeline(canvas,width,ratio,start,end,tracks,raw):drawDetailedTimeline(canvas,width,ratio,start,end,tracks,raw)}
  if(height===undefined){const result=await query(sql,10000,signal);height=state.timelineRenderer==='kutrace'?drawKutraceSummary(canvas,width,ratio,start,end,tracks,result,bucket,pixelPerBucket,useMipmap?'mipmap':'events',useMipmap?(useCoarse?'coarse':'fine'):'none'):drawSummaryTimeline(canvas,width,ratio,start,end,tracks,result,bucket,pixelPerBucket,useMipmap?'mipmap':'events',useMipmap?(useCoarse?'coarse':'fine'):'none')}
  installTimelineNavigation(canvas,width);renderSelectionOverlay();canvas.dataset.preview='false';canvas.dataset.ready='true';
}
function overviewCacheKey(){return JSON.stringify([state.full,state.filters,state.overlays,state.groups])}
async function drawOverview(signal){
  if(!state.full)return;const canvas=$('overview'),width=canvas.getBoundingClientRect().width||800,height=canvas.getBoundingClientRect().height||72,ratio=devicePixelRatio||1,[start,end]=state.full,bins=Math.max(64,Math.min(1000,Math.floor(width))),bucket=(end-start)/bins,key=overviewCacheKey();
  if(state.overviewKey!==key){const result=await query(`SELECT MIN(${bins-1},MAX(0,CAST((ts-${start})/${bucket} AS INTEGER))) AS bin,COUNT(*) FROM events ${where()} GROUP BY bin ORDER BY bin`,bins,signal);state.overviewRows=result.rows;state.overviewKey=key}
  canvas.width=Math.round(width*ratio);canvas.height=Math.round(height*ratio);const ctx=canvas.getContext('2d');ctx.scale(ratio,ratio);ctx.fillStyle='#080b11';ctx.fillRect(0,0,width,height);const counts=new Map(state.overviewRows.map(row=>[Number(row[0]),Number(row[1])])),max=Math.max(1,...counts.values());ctx.fillStyle=state.overlays.colorblind?'#56b4e9':'#5f91df';
  for(let bin=0;bin<bins;bin++){const count=counts.get(bin)||0,h=Math.max(count?1:0,(count/max)*(height-8));ctx.fillRect(bin*width/bins,height-h,Math.ceil(width/bins),h)}
  updateOverviewViewport();canvas.dataset.ready='true';
}
function updateOverviewViewport(){
  if(!state.full||!state.range)return;const span=state.full[1]-state.full[0],left=100*(state.range[0]-state.full[0])/span,width=100*(state.range[1]-state.range[0])/span,viewport=$('overview-viewport');viewport.style.left=`${Math.max(0,left)}%`;viewport.style.width=`${Math.min(100,width)}%`;
}
function installOverviewNavigation(){
  const canvas=$('overview');let drag=null;const fraction=event=>Math.max(0,Math.min(1,event.offsetX/Math.max(1,canvas.getBoundingClientRect().width)));
  canvas.onpointerdown=event=>{if(event.button!==0)return;drag={start:fraction(event),range:[...state.range],moved:false};canvas.setPointerCapture(event.pointerId);event.preventDefault()};
  canvas.onpointermove=event=>{if(!drag)return;const now=fraction(event);drag.moved=drag.moved||Math.abs(now-drag.start)>.003;if(drag.moved){const delta=(now-drag.start)*(state.full[1]-state.full[0]);state.range=boundedRange(drag.range[0]+delta,drag.range[1]+delta);updateOverviewViewport()}};
  canvas.onpointerup=event=>{if(!drag)return;const moved=drag.moved;drag=null;if(moved)refresh();else{const center=state.full[0]+fraction(event)*(state.full[1]-state.full[0]),span=state.range[1]-state.range[0];setRange(center-span/2,center+span/2)}};
}
function eventSearchSql(){
  if(!state.search)return '';const value=`'%' || ${literal(state.search)} || '%'`,predicate=`(name LIKE ${value} OR category LIKE ${value} OR CAST(pid AS TEXT) LIKE ${value} OR CAST(cpu AS TEXT) LIKE ${value} OR CAST(event AS TEXT) LIKE ${value})`;return state.searchInvert?`NOT ${predicate}`:predicate;
}
async function loadEvents(signal){
  const [start,end]=state.range,scope=`ts < ${end} AND ts_end > ${start}`,sql=`SELECT id,ts,dur,cpu,pid,event,name,category,arg0,retval,ipc FROM events ${where(scope)} ORDER BY ts LIMIT 501`,result=await query(sql,501,signal),rows=result.rows.slice(0,500),target=$('event-table');
  target.innerHTML=`<thead><tr>${['ts','dur','cpu','pid','event','name','category','arg0','retval','performance'].map(c=>`<th>${c}</th>`).join('')}</tr></thead><tbody>${rows.map(row=>{const item={id:row[0],start:row[1],end:row[1]+row[2],track:state.trackMode==='cpu_pid'?`cpu:${row[3]}`:(state.trackMode==='cpu'?row[3]:row[4]),event:row[5],name:row[6],category:row[7]},match=searchMatch(item);return `<tr data-event-id="${row[0]}"${state.selectedEvent&&state.selectedEvent.id===row[0]?' aria-selected="true"':''}${state.search?` style="opacity:${match?1:.32}"`:''}>${[...row.slice(1,10),performance(row[10])].map(v=>`<td>${escapeHtml(v??'')}</td>`).join('')}</tr>`}).join('')}</tbody>`;
  target.querySelectorAll('[data-event-id]').forEach((tr,index)=>tr.onclick=()=>{const row=rows[index];setSelection(row[1],row[1]+Math.max(row[2],1e-9),{id:row[0],start:row[1],end:row[1]+row[2],track:state.trackMode==='cpu_pid'?`cpu:${row[3]}`:(state.trackMode==='cpu'?row[3]:row[4]),event:row[5],name:row[6],category:row[7]})});
  $('event-count').textContent=result.rows.length>500?'500+ rows':`${result.rows.length} rows`;
  const searchExtra=eventSearchSql();if(searchExtra){const count=await query(`SELECT COUNT(*) FROM events ${where(`${scope} AND ${searchExtra}`)}`,1,signal);$('search-count').textContent=`${count.rows[0][0]} matches`;canvasSearchCount(count.rows[0][0])}else{$('search-count').textContent='';canvasSearchCount(0)}
}
function canvasSearchCount(count){$('timeline').dataset.searchCount=String(count);$('timeline').dataset.searchMatches=String(count)}
async function loadFlamegraph(signal){
  if(!state.range)return;const [start,end]=state.selection||state.range,weightMode=$('flame-weight').value,sql=`SELECT category,name,SUM(MAX(0,MIN(ts_end,${end})-MAX(ts,${start}))) AS duration,COUNT(*) AS count FROM events ${where(`ts < ${end} AND ts_end > ${start}`)} GROUP BY category,name ORDER BY category,name`,result=await query(sql,10000,signal),rows=result.rows.map(([category,name,duration,count])=>({category,name:name||'(unnamed)',duration:Number(duration||0),count:Number(count||0)}));
  $('flame-range').textContent=`${start.toFixed(6)}s – ${end.toFixed(6)}s`;const durationTotal=rows.reduce((sum,row)=>sum+row.duration,0),effective=weightMode==='duration'&&durationTotal>0?'duration':'count',total=rows.reduce((sum,row)=>sum+row[effective],0),target=$('flamegraph');
  if(!rows.length||!total){target.innerHTML='<span class="muted">No spans in this range.</span>';$('flame-status').textContent='0 spans';return}
  const categories=new Map();for(const row of rows){if(!categories.has(row.category))categories.set(row.category,[]);categories.get(row.category).push(row)}
  const frames=[`<button class="flame-frame" data-flame-frame data-flame-level="root" style="left:0%;top:2px;width:100%;background:#9fb7d7" title="root · ${total}">root · ${rows.reduce((sum,row)=>sum+row.count,0)} spans</button>`];let categoryOffset=0;const palette=state.overlays.colorblind?colorblindColors:colors;
  for(const [category,items] of categories){const categoryWeight=items.reduce((sum,row)=>sum+row[effective],0),categoryWidth=100*categoryWeight/total;if(categoryWidth<=0)continue;frames.push(`<button class="flame-frame" data-flame-frame data-flame-level="category" data-flame-category="${escapeHtml(category)}" style="left:${categoryOffset}%;top:31px;width:${categoryWidth}%;background:${palette[category]||palette.special}" title="${escapeHtml(category)} · ${categoryWeight}">${escapeHtml(category)}</button>`);let itemOffset=categoryOffset;for(const row of items){const itemWidth=100*row[effective]/total;if(itemWidth<=0)continue;frames.push(`<button class="flame-frame" data-flame-frame data-flame-level="name" data-flame-category="${escapeHtml(category)}" data-flame-name="${escapeHtml(row.name)}" style="left:${itemOffset}%;top:60px;width:${itemWidth}%;background:${palette[category]||palette.special}" title="${escapeHtml(row.name)} · ${row.count} spans · ${(row.duration*1000).toFixed(3)} ms">${escapeHtml(row.name)}</button>`);itemOffset+=itemWidth}categoryOffset+=categoryWidth}
  target.innerHTML=frames.join('');target.querySelector('[data-flame-level="root"]').onclick=()=>setRange(start,end);target.querySelectorAll('[data-flame-level="category"]').forEach(button=>button.onclick=()=>addExactFilter('category',button.dataset.flameCategory));target.querySelectorAll('[data-flame-level="name"]').forEach(button=>button.onclick=()=>addExactFilter('name',button.dataset.flameName));$('flame-status').textContent=`${rows.reduce((sum,row)=>sum+row.count,0)} spans · ${effective}`;
}
function addExactFilter(field,value){if(!state.filters.some(filter=>filter.field===field&&filter.op==='='&&filter.value===value))state.filters.push({field,op:'=',value});state.overviewKey='';renderChips();refresh()}
async function loadAgentTree(signal){
  const [start,end]=state.range,sql=`WITH RECURSIVE tree(id,span_id,parent_span_id,name,ts,dur,depth,path) AS (
    SELECT id,span_id,parent_span_id,name,ts,dur,0,printf('%020d',id) FROM agent_spans WHERE parent_span_id=0 AND ts<${end} AND ts_end>${start}
    UNION ALL SELECT child.id,child.span_id,child.parent_span_id,child.name,child.ts,child.dur,tree.depth+1,tree.path||'.'||printf('%020d',child.id) FROM agent_spans child JOIN tree ON child.parent_span_id=tree.span_id WHERE tree.depth<32)
    SELECT depth,name,ROUND(ts,8),ROUND(dur*1000,3),CAST(span_id AS TEXT),CAST(parent_span_id AS TEXT),(SELECT COUNT(*) FROM agent_annotations annotation WHERE annotation.span_id=tree.span_id) FROM tree ORDER BY path LIMIT 1000`,result=await query(sql,1000,signal),spanIds=new Set(result.rows.map(row=>row[4]));
  if(!spanIds.has(state.selectedAgent)){state.selectedAgent=result.rows.length?result.rows[0][4]:null;state.selectedAgentName=result.rows.length?result.rows[0][1]:''}else{const selected=result.rows.find(row=>row[4]===state.selectedAgent);if(selected)state.selectedAgentName=selected[1]}
  $('agent-tree').innerHTML=result.rows.length?result.rows.map(row=>`<button class="agent-node${row[4]===state.selectedAgent?' selected':''}" style="padding-left:${4+row[0]*12}px" data-agent-span="${row[4]}"><span>${escapeHtml(row[1])}</span> ${row[3]}ms · #${row[4]}${row[6]?` · ${row[6]} note${row[6]===1?'':'s'}`:''}</button>`).join(''):'<span class="muted">No root agent spans in this range.</span>';
  document.querySelectorAll('[data-agent-span]').forEach(button=>button.onclick=async()=>{state.selectedAgent=button.dataset.agentSpan;state.selectedAgentName=button.querySelector('span').textContent;document.querySelectorAll('[data-agent-span]').forEach(node=>node.classList.toggle('selected',node===button));try{await loadAgentContext()}catch(error){showError(error)}});await loadAgentContext(signal);
}
async function loadAgentContext(signal){
  if(state.selectedAgent===null){state.selectedAgentSql='';$('open-agent-context').disabled=true;$('agent-context-title').textContent='Select an agent span';$('agent-context-count').textContent='';$('agent-context-sql').textContent='';$('agent-context-table').innerHTML='';$('agent-annotations').className='annotation-list muted';$('agent-annotations').textContent='No linked query, observation, decision, or result annotations.';return}
  if(!/^-?\d+$/.test(state.selectedAgent))throw new Error('Invalid agent span id');const spanId=state.selectedAgent,sql=`WITH selected AS (
  SELECT id,span_id,name,ts,ts_end,pid FROM agent_spans WHERE span_id=${spanId} LIMIT 1
), context AS (
  SELECT 'annotation' AS source,a.ts,a.dur,a.pid,0 AS rpc,a.event,a.value AS arg0,a.span_id AS retval,a.annotation_kind AS category,a.name FROM agent_annotations a JOIN selected s ON a.span_id=s.span_id
  UNION ALL SELECT 'trace' AS source,e.ts,e.dur,e.pid,e.rpc,e.event,e.arg0,e.retval,e.category,e.name FROM events e JOIN selected s ON e.ts<s.ts_end AND e.ts_end>s.ts WHERE e.id!=s.id AND NOT(e.event BETWEEN 522 AND 525 AND e.retval=s.span_id)
) SELECT source,ROUND(ts,8) AS ts,ROUND(dur*1000,3) AS duration_ms,pid,rpc,event,arg0,retval,category,name FROM context ORDER BY ts,source LIMIT 200`;
  state.selectedAgentSql=sql;$('open-agent-context').disabled=false;$('agent-context-sql').textContent=sql;const result=await query(sql,200,signal),annotations=result.rows.filter(row=>row[0]==='annotation');$('agent-context-title').textContent=`${state.selectedAgentName} · span #${spanId}`;$('agent-context-count').textContent=`${result.rows.length} context rows · ${annotations.length} annotations`;const target=$('agent-annotations');
  if(annotations.length){target.className='annotation-list';target.innerHTML=annotations.map(row=>`<span class="annotation-pill"><b>${escapeHtml(row[8])}</b> ${escapeHtml(row[9])} · ${escapeHtml(row[6])}</span>`).join('')}else{target.className='annotation-list muted';target.textContent='No linked query, observation, decision, or result annotations.'}renderTable($('agent-context-table'),result);
}
async function loadRpcFlows(signal){const [start,end]=state.range,result=await query(`SELECT rpc_id,ROUND(MIN(ts),6),ROUND((MAX(ts_end)-MIN(ts))*1000,3),COUNT(*),GROUP_CONCAT(DISTINCT name) FROM rpc_activity ${where(`ts < ${end} AND ts_end > ${start}`)} GROUP BY rpc_id ORDER BY MIN(ts) LIMIT 200`,200,signal);$('rpc-flows').innerHTML=result.rows.length?result.rows.map(row=>`<div class="relation-row"><b>RPC ${row[0]}</b> · ${row[2]}ms · ${row[3]} events<br>${escapeHtml(row[4])}</div>`).join(''):'<span class="muted">No RPC activity in this range.</span>'}
async function loadResourceActivity(signal){const [start,end]=state.range,result=await query(`SELECT ROUND(ts,6),ROUND(dur*1000,3),pid,rpc,event,arg0,name,category FROM resource_activity ${where(`ts < ${end} AND ts_end > ${start}`)} ORDER BY ts LIMIT 200`,200,signal);$('resource-activity').innerHTML=result.rows.length?result.rows.map(row=>`<div class="relation-row"><b>${escapeHtml(row[6])}</b> · ${row[1]}ms<br>${row[7]} #${row[5]} · PID ${row[2]}${row[3]?` · RPC ${row[3]}`:''}</div>`).join(''):'<span class="muted">No resource activity in this range.</span>'}
function showError(error){if(error&&error.name!=='AbortError')$('query-status').textContent=error.message||String(error)}
async function refresh(){
  if(!state.range)return;if(state.refreshController)state.refreshController.abort();const controller=new AbortController();state.refreshController=controller;
  try{await Promise.all([drawTimeline(controller.signal),drawOverview(controller.signal),loadEvents(controller.signal),loadFlamegraph(controller.signal),loadAgentTree(controller.signal),loadRpcFlows(controller.signal),loadResourceActivity(controller.signal)])}catch(error){showError(error)}finally{if(state.refreshController===controller)state.refreshController=null}
}
let sqlController=null;
async function runSql(sql=$('sql').value){if(sqlController)sqlController.abort();const controller=new AbortController();sqlController=controller;$('query-status').textContent='Running…';try{const result=await query(sql,1000,controller.signal);if(sqlController!==controller)return;renderTable($('query-table'),result);$('query-status').textContent=`${result.rows.length}${result.truncated?' (truncated)':''} rows · ${result.elapsed_ms.toFixed(2)} ms · exact SQL shown above`}catch(error){if(error.name!=='AbortError'&&sqlController===controller)showError(error)}}
function activateDock(name){state.activeDock=name;document.querySelectorAll('.dock-tab').forEach(tab=>{const active=tab.dataset.dock===name;tab.classList.toggle('active',active);tab.setAttribute('aria-selected',String(active))});document.querySelectorAll('[data-dock-panel]').forEach(panel=>panel.classList.toggle('active',panel.dataset.dockPanel===name));document.querySelector('.analysis-dock').classList.remove('collapsed');$('timeline-view').classList.add('dock-open');$('toggle-dock').textContent='⌄';$('toggle-dock').setAttribute('aria-expanded','true')}
function syncWorkspaceChrome(){document.body.classList.toggle('kutrace-primary',state.activeView==='legacy')}
function activateTimelineRenderer(name){
  state.timelineRenderer=name==='lanes'?'lanes':'kutrace';document.querySelectorAll('.renderer-tab').forEach(tab=>{const active=tab.dataset.renderer===state.timelineRenderer;tab.classList.toggle('active',active);tab.setAttribute('aria-selected',String(active))});$('renderer-label').textContent=state.timelineRenderer==='kutrace'?'Native KUtrace':'Event lanes';syncWorkspaceChrome();if(state.full){$('timeline').dataset.ready='false';refresh()}
}
function activateView(name){state.activeView=name;document.querySelectorAll('.view-tab').forEach(tab=>{const active=tab.dataset.view===name;tab.classList.toggle('active',active);tab.setAttribute('aria-selected',String(active))});document.querySelectorAll('.view-pane').forEach(pane=>{const active=pane.id===`${name}-view`;pane.classList.toggle('active',active);pane.hidden=!active});if(name==='legacy'&&!$('legacy-frame').hasAttribute('src'))$('legacy-frame').src=$('legacy-frame').dataset.src;syncWorkspaceChrome();if(name==='timeline'&&state.timelineRenderer==='lanes')refresh()}
function applyLegacyRange(legacy,left,width){
  if(!legacy.x||!legacy.d3||!legacy.state2||typeof legacy.do_zoomed_x2!=='function')return false;const [pixelStart,pixelEnd]=legacy.x.range(),[timeStart,timeEnd]=legacy.x.domain(),fullWidth=timeEnd-timeStart,multiplier=(pixelEnd-pixelStart)/fullWidth,scale=fullWidth/width,offset=-(left-timeStart)*scale*multiplier-pixelStart*(scale-1),transform=legacy.d3.zoomIdentity.translate(offset,legacy.state2.savedtransformX.y).scale(scale);legacy.state2.savedtransformX=transform;if(legacy.state)legacy.state.just_reset=false;const surface=legacy.panzoomrect_x&&legacy.panzoomrect_x.node();if(surface)surface.__zoom=transform;legacy.do_zoomed_x2(transform,[(pixelStart+pixelEnd)/2,0]);return true;
}
function navigateLegacy(action){
  const frame=$('legacy-frame'),legacy=frame.contentWindow;if(!legacy||typeof legacy.do_zoomed_x2!=='function')return false;
  const fullStart=Number(legacy.dataTsLo),fullEnd=Number(legacy.dataTsHi),currentStart=Number(legacy.realxleft),currentEnd=Number(legacy.realxright);if(![fullStart,fullEnd,currentStart,currentEnd].every(Number.isFinite)||fullEnd<=fullStart)return false;
  const fullSpan=fullEnd-fullStart,currentSpan=Math.max(Number.EPSILON,currentEnd-currentStart);let width=currentSpan,left=currentStart;
  if(action==='reset-range'){left=fullStart;width=fullSpan}
  else if(action==='zoom-in'){width=Math.max(fullSpan*1e-9,currentSpan*.5);left=(currentStart+currentEnd-width)/2}
  else if(action==='zoom-out'){width=Math.min(fullSpan,currentSpan*2);left=(currentStart+currentEnd-width)/2}
  else if(action==='pan-left')left-=currentSpan*.25;
  else if(action==='pan-right')left+=currentSpan*.25;
  else return false;
  if(width>=fullSpan){left=fullStart;width=fullSpan}else left=Math.max(fullStart,Math.min(fullEnd-width,left));return applyLegacyRange(legacy,left,width);
}
function runNavigation(action){
  if(state.activeView==='legacy')return navigateLegacy(action);
  if(action==='reset-range')return setRange(...state.full);if(action==='zoom-in')return zoomRange(.5);if(action==='zoom-out')return zoomRange(2);if(action==='pan-left')return panRange(-.25);if(action==='pan-right')return panRange(.25);return false;
}
const navigationKeys={w:'zoom-in',s:'zoom-out',a:'pan-left',d:'pan-right','+':'zoom-in','=':'zoom-in','-':'zoom-out','0':'reset-range',Home:'reset-range',ArrowLeft:'pan-left','[':'pan-left',ArrowRight:'pan-right',']':'pan-right'};
function handleNavigationKey(event){const target=event.target;if((target&&typeof target.matches==='function'&&target.matches('input,textarea,select,[contenteditable=true]'))||event.ctrlKey||event.metaKey||event.altKey)return;const key=event.key.length===1?event.key.toLowerCase():event.key,action=navigationKeys[key];if(action){event.preventDefault();runNavigation(action)}}
function installLegacyNavigation(frameId){const frame=$(frameId);frame.addEventListener('load',()=>{try{const legacy=frame.contentWindow;if(!legacy||legacy.__kutraceKeyboardNavigation)return;legacy.__kutraceKeyboardNavigation=true;legacy.addEventListener('keydown',handleNavigationKey)}catch(error){showError(error)}})}
function syncToggleUi(){
  document.documentElement.classList.toggle('colorblind',state.overlays.colorblind);document.body.classList.toggle('colorblind',state.overlays.colorblind);document.querySelectorAll('[data-overlay]').forEach(button=>button.setAttribute('aria-pressed',String(state.overlays[button.dataset.overlay])));document.querySelectorAll('[data-track-group]').forEach(button=>button.classList.toggle('active',state.groups[button.dataset.trackGroup]));updatePerformanceLegend();
}
async function pollExtent(){
  try{const result=await query('SELECT COUNT(*),MIN(ts),MAX(ts_end) FROM events',1),[rawCount,rawStart,rawEnd]=result.rows[0],count=Number(rawCount||0),start=Number(rawStart||state.full[0]),end=Number(rawEnd||state.full[1]);if(count===state.eventCount&&start===state.full[0]&&end===state.full[1])return;$('live-status').classList.add('changed');$('live-status').textContent=`${count} events · ${state.followTail?'following live':'updated'}`;const oldRange=[...state.range],span=oldRange[1]-oldRange[0];state.eventCount=count;state.full=[start,Math.max(start+1e-9,end)];state.overviewKey='';if(state.followTail)state.range=boundedRange(state.full[1]-span,state.full[1]);await refresh()}catch(error){showError(error)}
}

$('add-filter').onclick=()=>{const value=$('filter-value').value.trim();if(!value)return;state.filters.push({field:$('filter-field').value,op:$('filter-op').value,value});$('filter-value').value='';state.overviewKey='';renderChips();refresh()};
$('run-sql').onclick=()=>runSql();$('show-schema').onclick=()=>{activateDock('sql');fetch('/api/schema').then(r=>r.json()).then(r=>{renderTable($('query-table'),r);$('query-status').textContent='sqlite_schema'}).catch(showError)};
$('open-agent-context').onclick=()=>{if(!state.selectedAgentSql)return;$('sql').value=state.selectedAgentSql;$('view-name').value=`${state.selectedAgentName} context`;activateDock('sql');runSql()};
$('save-view').onclick=()=>{const name=$('view-name').value.trim(),sql=$('sql').value.trim();if(!name){$('query-status').textContent='Name the SQL view before saving';return}if(!sql){$('query-status').textContent='Cannot save an empty query';return}const existing=state.views.findIndex(view=>view.name===name);if(existing>=0)state.views[existing]={name,sql};else if(state.views.length>=32){$('query-status').textContent='Saved SQL view limit is 32';return}else state.views.push({name,sql});renderSavedViews();$('query-status').textContent=existing>=0?`Updated saved view “${name}”`:`Saved view “${name}”`};
$('reset-range').onclick=()=>runNavigation('reset-range');$('zoom-in').onclick=()=>runNavigation('zoom-in');$('zoom-out').onclick=()=>runNavigation('zoom-out');$('pan-left').onclick=()=>runNavigation('pan-left');$('pan-right').onclick=()=>runNavigation('pan-right');$('zoom-selection').onclick=()=>state.selection&&setRange(...state.selection);$('clear-selection').onclick=clearSelection;
$('previous-cpus').onclick=()=>{if(state.cpuPage>0){state.cpuPage--;refresh()}};$('next-cpus').onclick=()=>{state.cpuPage++;refresh()};
$('track-mode').onchange=event=>{state.trackMode=event.target.value;state.cpuPage=0;state.overviewKey='';refresh()};
document.querySelectorAll('[data-overlay]').forEach(button=>button.onclick=()=>{const key=button.dataset.overlay;state.overlays[key]=!state.overlays[key];state.overviewKey='';syncToggleUi();refresh()});
document.querySelectorAll('[data-track-group]').forEach(button=>button.onclick=()=>{const key=button.dataset.trackGroup;state.groups[key]=!state.groups[key];button.textContent=(state.groups[key]?'▾':'▸')+button.textContent.slice(1);state.overviewKey='';syncToggleUi();refresh()});
document.querySelectorAll('.view-tab').forEach(button=>button.onclick=()=>activateView(button.dataset.view));document.querySelectorAll('.renderer-tab').forEach(button=>button.onclick=()=>activateTimelineRenderer(button.dataset.renderer));document.querySelectorAll('.dock-tab').forEach(button=>button.onclick=()=>activateDock(button.dataset.dock));
$('toggle-dock').onclick=()=>{const dock=document.querySelector('.analysis-dock'),collapsed=dock.classList.toggle('collapsed');$('timeline-view').classList.toggle('dock-open',!collapsed);$('toggle-dock').textContent=collapsed?'⌃':'⌄';$('toggle-dock').setAttribute('aria-expanded',String(!collapsed))};
let searchTimer=null;$('trace-search').oninput=event=>{state.search=event.target.value.trim();clearTimeout(searchTimer);searchTimer=setTimeout(()=>refresh(),120)};$('search-invert').onclick=()=>{state.searchInvert=!state.searchInvert;$('search-invert').setAttribute('aria-pressed',String(state.searchInvert));refresh()};
$('flame-weight').onchange=()=>loadFlamegraph().catch(showError);$('follow-live').onclick=()=>{state.followTail=!state.followTail;$('follow-live').setAttribute('aria-pressed',String(state.followTail));$('live-status').textContent=`${state.eventCount} events · ${state.followTail?'following':'static'}`;if(state.followTail){const span=state.range[1]-state.range[0];setRange(state.full[1]-span,state.full[1])}};
$('save-workspace').onclick=()=>{localStorage.setItem('kutrace-workspace',JSON.stringify(workspaceState()));$('query-status').textContent='Workspace saved locally'};
$('export-workspace').onclick=()=>{const url=URL.createObjectURL(new Blob([JSON.stringify(workspaceState(),null,2)],{type:'application/json'})),link=document.createElement('a');link.href=url;link.download='kutrace-workspace.json';link.click();URL.revokeObjectURL(url);$('query-status').textContent='Workspace exported'};$('import-workspace').onclick=()=>$('workspace-file').click();
$('workspace-file').onchange=async event=>{try{const file=event.target.files[0];if(!file)return;applyWorkspace(JSON.parse(await file.text()));localStorage.setItem('kutrace-workspace',JSON.stringify(workspaceState()));state.overviewKey='';await refresh();await runSql();$('query-status').textContent='Workspace imported'}catch(error){showError(error)}finally{event.target.value=''}};
window.addEventListener('keydown',handleNavigationKey);
let resizeTimer=null;window.addEventListener('resize',()=>{clearTimeout(resizeTimer);resizeTimer=setTimeout(()=>refresh(),100)});
(async()=>{installLegacyNavigation('legacy-frame');await loadMetadata();const saved=JSON.parse(localStorage.getItem('kutrace-workspace')||'null');if(saved)applyWorkspace(saved);else{activateTimelineRenderer('kutrace');renderChips();renderSavedViews();syncToggleUi()}installOverviewNavigation();renderSelectionSummary();await refresh();await runSql();setInterval(pollExtent,1500)})().catch(showError);
