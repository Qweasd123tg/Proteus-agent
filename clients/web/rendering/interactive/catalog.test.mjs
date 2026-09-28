import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {parseSpec} from './catalog.mjs';
const skill=readFileSync(new URL('../../../../configs/skills/interactive-response/SKILL.md',import.meta.url),'utf8');
export const fixture=skill.match(/```json-render\n([\s\S]*?)\n```/)[1];
test('shipped skill example validates against the actual json-render catalog',()=>{assert.equal(parseSpec(fixture).root,'summary');});
test('malformed graph and unsupported behavior keep source instead of partial rendering',()=>{
  for(const mutate of [
    s=>s.elements.chart.type='Unknown',
    s=>s.elements.chart.props.extra=true,
    s=>s.elements.chart.props.items[0].value=-1,
    s=>s.elements.views.children=['summary','table'],
    s=>s.elements.views.children=['absent','table'],
    s=>s.elements.views.children=['chart','chart'],
    s=>s.elements.views.children=['chart'],
    s=>s.elements.table.props.rows[0].pop(),
    s=>s.elements.chart.props.title={'$state':'/secret'},
    s=>s.elements.chart.on={click:{action:'run'}},
    s=>s.state={},
  ]) {const spec=JSON.parse(fixture);mutate(spec);assert.throws(()=>parseSpec(JSON.stringify(spec)));}
  assert.throws(()=>parseSpec('{unfinished'));
  assert.throws(()=>parseSpec(' '.repeat(200001)));
});
