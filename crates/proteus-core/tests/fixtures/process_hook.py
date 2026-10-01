"""Explicit process hook fixture: blocking and after-side-effect failure."""
import json, sys, time
mode, module_id, component_id = sys.argv[1:4]
marker = sys.argv[4] if len(sys.argv) > 4 else None
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = dict(protocol_version='v3', component_id=component_id, exports=[dict(slot='hook', module_id=module_id, contract_version='v2', composition='ordered_many', module_features=[])])
    else:
        event = request['params']['params']['event']['event']
        if mode == 'after_wait' and event == 'after_tool':
            if marker:
                with open(marker, 'w') as out:
                    out.write('after tool entered')
            time.sleep(3)
        if mode == 'after_error' and event == 'after_tool':
            print(json.dumps(dict(jsonrpc='2.0', id=request['id'], error=dict(code=-32000,message='fixture after-tool failed'))), flush=True)
            continue
        result = dict(result=dict(action='block_tool', reason='owner blocked tool') if mode == 'block' and event == 'before_tool' else dict(action='continue'))
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
