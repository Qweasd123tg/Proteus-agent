const local = path => new URL('../vendor/'+path,import.meta.url).href;
const libraries = new Map();
function once(name, load) {
  if(!libraries.has(name)) libraries.set(name,load().catch(error=>{libraries.delete(name);throw error;}));
  return libraries.get(name);
}
function script(url) {
  return new Promise((resolve,reject)=>{
    const element=document.createElement('script');element.src=url;
    element.onload=()=>resolve();element.onerror=()=>{element.remove();reject(new Error('Не удалось загрузить рендерер'));};
    document.head.append(element);
  });
}
export const highlight = () => once('highlight',async()=>{
  const style=document.createElement('link');style.rel='stylesheet';style.href=local('highlight/theme.css');document.head.append(style);
  await script(local('highlight/highlight.min.js'));return window.hljs;
});
export const math = () => once('math',async()=>{
  window.MathJax={
    startup:{typeset:false},
    loader:{paths:{mathjax:local('mathjax')},load:['ui/safe']},
    tex:{inlineMath:[['\\(','\\)']],displayMath:[['\\[','\\]']],processEscapes:true},
    chtml:{fontURL:local('mathjax/output/chtml/fonts/woff-v2')},
    options:{enableMenu:false},
  };
  await script(local('mathjax/tex-chtml.js'));
  await window.MathJax.startup.promise;return window.MathJax;
});
export const diagrams = () => once('mermaid',async()=>{
  const {default:mermaid}=await import(local('mermaid/mermaid.esm.min.mjs'));
  mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'base',fontFamily:'system-ui, sans-serif',
    themeVariables:{background:'#202020',primaryColor:'#303030',primaryTextColor:'#dedede',primaryBorderColor:'#666',lineColor:'#999',secondaryColor:'#383838',tertiaryColor:'#292929'}});
  return mermaid;
});
