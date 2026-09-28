import {parseSpec} from './catalog.mjs';
const element=(tag,cls,text)=>{
  const node=document.createElement(tag);if(cls)node.className=cls;if(text!==undefined)node.textContent=String(text??'—');return node;
};
function table(props) {
  const root=element('div','jr-table');
  const scroll=element('div','jr-table-scroll'), grid=element('table'), head=element('thead'), header=element('tr'),body=element('tbody');
  props.columns.forEach(label=>{const cell=element('th','',label);cell.scope='col';header.append(cell);});
  head.append(header);grid.append(head,body);scroll.append(grid);
  const draw=query=>{
    const rows=props.rows.filter(row=>row.some(cell=>String(cell??'').toLocaleLowerCase().includes(query)));
    body.replaceChildren(...rows.map(row=>{const tr=element('tr');row.forEach(value=>tr.append(element('td','',value)));return tr;}));
    if(!rows.length){const td=element('td','','Нет совпадений');td.colSpan=props.columns.length;const tr=element('tr');tr.append(td);body.append(tr);}
  };
  if(props.filterable){const search=element('input','jr-search');search.type='search';search.placeholder='Фильтр таблицы…';search.setAttribute('aria-label','Фильтр таблицы');search.addEventListener('input',()=>draw(search.value.toLocaleLowerCase()));root.append(search);}
  draw('');root.append(scroll);return root;
}
function chart(props) {
  const root=element('figure','jr-chart');root.append(element('figcaption','jr-title',props.title));
  const max=Math.max(...props.items.map(item=>item.value),1);
  for(const item of props.items){
    const row=element('div','jr-bar-row'),track=element('div','jr-bar-track'),bar=element('div','jr-bar');
    bar.style.width=(item.value/max*100)+'%';track.setAttribute('aria-hidden','true');track.append(bar);
    row.append(element('span','jr-bar-label',item.label),track,element('span','jr-bar-value',item.value+(props.unit?' '+props.unit:'')));root.append(row);
  }
  return root;
}
let nextTabs=0;
function tabs(props,children) {
  const root=element('div','jr-tabs'),list=element('div','jr-tab-list'),id='jr-tabs-'+(++nextTabs);
  list.setAttribute('role','tablist');list.setAttribute('aria-label','Представления');
  const buttons=props.labels.map((label,index)=>{
    const button=element('button','jr-tab',label);button.type='button';button.id=id+'-tab-'+index;button.setAttribute('role','tab');button.setAttribute('aria-controls',id+'-panel-'+index);return button;
  });
  const panels=children.map((child,index)=>{
    const panel=element('div','jr-tab-panel');panel.id=id+'-panel-'+index;panel.setAttribute('role','tabpanel');panel.setAttribute('aria-labelledby',buttons[index].id);panel.tabIndex=0;panel.append(child);return panel;
  });
  const select=index=>{buttons.forEach((button,i)=>{button.setAttribute('aria-selected',String(i===index));button.tabIndex=i===index?0:-1;panels[i].hidden=i!==index;});};
  buttons.forEach((button,index)=>{
    button.addEventListener('click',()=>select(index));
    button.addEventListener('keydown',event=>{
      const moves={ArrowRight:(index+1)%buttons.length,ArrowLeft:(index+buttons.length-1)%buttons.length,Home:0,End:buttons.length-1};
      if(Object.hasOwn(moves,event.key)){event.preventDefault();select(moves[event.key]);buttons[moves[event.key]].focus();}
    });
  });
  select(0);list.append(...buttons);root.append(list,...panels);return root;
}
export function render(source) {
  const spec=parseSpec(source);
  const draw=key=>{
    const {type,props,children}=spec.elements[key];
    switch(type){
      case 'Text':return element('p','jr-text',props.text);
      case 'Metric':{const root=element('div','jr-metric');root.append(element('span','jr-muted',props.label),element('strong','jr-metric-value',props.value));if(props.detail)root.append(element('span','jr-muted',props.detail));return root;}
      case 'Table':return table(props);
      case 'BarChart':return chart(props);
      case 'Tabs':return tabs(props,children.map(draw));
      default:{const root=element('div',type==='Card'?'jr-card':'jr-stack');if(type==='Card')root.append(element('div','jr-title',props.title));root.append(...children.map(draw));return root;}
    }
  };
  const root=element('div','markdown-interactive');root.append(draw(spec.root));return root;
}
