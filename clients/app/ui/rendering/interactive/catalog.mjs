import {defineSchema, defineCatalog} from '@json-render/core';
import {z} from 'zod';

const text = z.string().max(8000);
const label = z.string().min(1).max(160);
const cell = z.union([text,z.number().finite(),z.boolean(),z.null()]);
const component = (props, description) => ({props:z.strictObject(props),description});
export const components = {
  Stack: component({}, 'Вертикальная группа'),
  Card: component({title:label}, 'Карточка с заголовком'),
  Text: component({text}, 'Обычный текст'),
  Metric: component({label,value:cell,detail:text.optional()}, 'Показатель'),
  Table: component({columns:z.array(label).min(1).max(12),rows:z.array(z.array(cell).max(12)).max(200),filterable:z.boolean().optional()}, 'Таблица с локальным поиском'),
  BarChart: component({title:label,items:z.array(z.strictObject({label,value:z.number().finite().min(0)})).min(1).max(40),unit:z.string().max(40).optional()}, 'Горизонтальные столбцы'),
  Tabs: component({labels:z.array(label).min(1).max(8)}, 'Локальные вкладки, один дочерний элемент на вкладку'),
};
const schema = defineSchema(s=>({
  spec:s.object({root:s.string(),elements:s.record(s.object({type:s.ref('catalog.components'),props:s.propsOf('catalog.components'),children:s.array(s.string())}))}),
  catalog:s.object({components:s.map({props:s.zod(),description:s.string()})}),
}));
export const catalog = defineCatalog(schema,{components});

export function parseSpec(source) {
  if(source.length>200000)throw Error('Слишком большой блок: максимум 200 КБ');
  const raw=JSON.parse(source);
  // The adapter deliberately supports only the documented tree, without actions/bindings.
  const shape=z.strictObject({root:z.string(),elements:z.record(z.string(),z.strictObject({type:z.enum(Object.keys(components)),props:z.record(z.string(),z.unknown()),children:z.array(z.string())}))});
  shape.parse(raw);
  const checked=catalog.validate(raw);
  if(!checked.success)throw Error('Некорректный компонент: '+checked.error.message);
  const spec=checked.data, keys=Object.keys(spec.elements);
  if(keys.length>100)throw Error('Максимум 100 компонентов');
  const seen=new Set();
  function visit(key,depth) {
    if(depth>12||seen.has(key))throw Error('Дерево содержит цикл, повтор или слишком глубокую вложенность');
    if(!Object.hasOwn(spec.elements,key))throw Error('Отсутствует компонент '+key);
    seen.add(key);
    const node=spec.elements[key];
    // Validate literal props separately: core also supports expressions in other renderers.
    components[node.type].props.parse(raw.elements[key].props);
    if(!['Stack','Card','Tabs'].includes(node.type)&&node.children.length)throw Error('Этот компонент не принимает children');
    if(node.type==='Tabs'&&node.props.labels.length!==node.children.length)throw Error('Количество вкладок и children должно совпадать');
    if(node.type==='Table'&&node.props.rows.some(row=>row.length!==node.props.columns.length))throw Error('Число ячеек должно совпадать с columns');
    node.children.forEach(child=>visit(child,depth+1));
  }
  visit(spec.root,0);
  if(seen.size!==keys.length)throw Error('Есть компоненты вне корневого дерева');
  return spec;
}
