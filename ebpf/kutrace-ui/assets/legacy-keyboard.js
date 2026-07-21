(()=>{
  if(window.__kutraceKeyboardNavigation)return;
  window.__kutraceKeyboardNavigation=true;
  const actions={w:'zoom-in',s:'zoom-out',a:'pan-left',d:'pan-right','+':'zoom-in','=':'zoom-in','-':'zoom-out','0':'reset-range',Home:'reset-range',ArrowLeft:'pan-left','[':'pan-left',ArrowRight:'pan-right',']':'pan-right'};
  function applyRange(left,width){
    if(!window.x||!window.d3||!window.state2||typeof window.do_zoomed_x2!=='function')return;
    const [pixelStart,pixelEnd]=x.range(),[timeStart,timeEnd]=x.domain(),fullWidth=timeEnd-timeStart,multiplier=(pixelEnd-pixelStart)/fullWidth,scale=fullWidth/width,offset=-(left-timeStart)*scale*multiplier-pixelStart*(scale-1),transform=d3.zoomIdentity.translate(offset,state2.savedtransformX.y).scale(scale);
    state2.savedtransformX=transform;if(window.state)state.just_reset=false;const surface=window.panzoomrect_x&&panzoomrect_x.node();if(surface)surface.__zoom=transform;do_zoomed_x2(transform,[(pixelStart+pixelEnd)/2,0]);
  }
  function navigate(action){
    const fullStart=Number(window.dataTsLo),fullEnd=Number(window.dataTsHi),currentStart=Number(window.realxleft),currentEnd=Number(window.realxright);if(![fullStart,fullEnd,currentStart,currentEnd].every(Number.isFinite)||fullEnd<=fullStart)return;
    const fullSpan=fullEnd-fullStart,currentSpan=Math.max(Number.EPSILON,currentEnd-currentStart);let width=currentSpan,left=currentStart;
    if(action==='reset-range'){left=fullStart;width=fullSpan}
    else if(action==='zoom-in'){width=Math.max(fullSpan*1e-9,currentSpan*.5);left=(currentStart+currentEnd-width)/2}
    else if(action==='zoom-out'){width=Math.min(fullSpan,currentSpan*2);left=(currentStart+currentEnd-width)/2}
    else if(action==='pan-left')left-=currentSpan*.25;
    else if(action==='pan-right')left+=currentSpan*.25;
    if(width>=fullSpan){left=fullStart;width=fullSpan}else left=Math.max(fullStart,Math.min(fullEnd-width,left));applyRange(left,width);
  }
  window.addEventListener('keydown',event=>{const target=event.target;if((target&&typeof target.matches==='function'&&target.matches('input,textarea,select,[contenteditable=true]'))||event.ctrlKey||event.metaKey||event.altKey)return;const key=event.key.length===1?event.key.toLowerCase():event.key,action=actions[key];if(action){event.preventDefault();navigate(action)}});
})();
