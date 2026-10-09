"""Browser-only host adapter. Real extraction/IPC/protocol is gated by native smoke."""
import base64
from io import BytesIO
import json
import mimetypes
from urllib.request import urlopen
from uuid import uuid4
import zipfile

BOOTSTRAP = """<script>
window.__TAURI__={core:{async invoke(command,args){
  const action={install_ui_extension:'install',remove_ui_extension:'remove'}[command];
  if(!action)throw Error('Unsupported browser fixture IPC: '+command);
  const response=await fetch('/fixture-packages/'+action,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(args)});
  const result=await response.json();if(!response.ok)throw Error(result.error);return result;
}}};
</script>"""
BOOTSTRAP += "<script type='module'>import '/extensions/web-adapter.js';delete window.__TAURI__;</script>"


def post(handler):
    if not handler.path.startswith('/fixture-packages/'):
        return False
    try:
        args = json.loads(handler.rfile.read(int(handler.headers['Content-Length'])))
        if handler.path == '/fixture-packages/install':
            with zipfile.ZipFile(BytesIO(base64.b64decode(args['archive']))) as archive:
                files = {name: archive.read(name) for name in archive.namelist()}
            manifest = json.loads(files['extension.json'])
            assert manifest['apiVersion'] == 4 and manifest['id'] not in args['excludedIds'], 'Already installed or invalid manifest'
            key = str(uuid4())
            handler.server.extension_packages[key] = files
            result = {'id': manifest['id'], 'key': key, 'url': f'http://127.0.0.1:{handler.server.server_port}/fixture-packages/{key}/extension.json'}
        else:
            handler.server.extension_packages.pop(args['key'], None)
            result = None
        handler.send_response(200)
    except Exception as error:
        handler.send_response(400)
        result = {'error': str(error)}
    handler.send_header('Content-Type', 'application/json')
    handler.end_headers()
    handler.wfile.write(json.dumps(result).encode())
    return True


def get(handler, root):
    if handler.path.startswith('/fixture-packages/'):
        key, resource = handler.path.removeprefix('/fixture-packages/').split('/', 1)
        content = handler.server.extension_packages.get(key, {}).get(resource)
        if content is None:
            handler.send_error(404)
        else:
            handler.send_response(200)
            handler.send_header('Content-Type', mimetypes.guess_type(resource)[0] or 'application/octet-stream')
            handler.end_headers()
            handler.wfile.write(content)
        return True
    if handler.path.startswith('/fixture/') and handler.path.endswith('.zip'):
        base = f'http://127.0.0.1:{handler.server.server_port}'
        folder = handler.path.removesuffix('package.zip')
        with urlopen(base + folder + 'extension.json') as response:
            manifest = json.load(response)
        files = {}
        for view in manifest['views']:
            entry = view['entry'].removeprefix('./')
            with urlopen(base + folder + entry) as response:
                files['lib/' + entry] = response.read()
            files[entry] = ("export {mount} from './lib/" + entry + "';").encode()
        manifest['icon'] = {'src': './assets/icon.svg'}
        if manifest.get('preview'):
            entry = manifest['preview']['entry'].removeprefix('./')
            with urlopen(base + folder + entry) as response:
                files['lib/' + entry] = response.read()
            files[entry] = ("export {createServices} from './lib/" + entry + "';").encode()
        files['assets/icon.svg'] = (root / 'clients/app/ui/extensions/notes/assets/icon.svg').read_bytes()
        files['extension.json'] = json.dumps(manifest).encode()
        output = BytesIO()
        with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for name, content in files.items():
                archive.writestr(name, content)
        handler.send_response(200)
        handler.send_header('Content-Type', 'application/zip')
        handler.end_headers()
        handler.wfile.write(output.getvalue())
        return True
    return False


def install(js, path):
    assert js("return !document.querySelector('.extension-install input[type=url]') && !document.querySelector('.extension-install input[type=file]').disabled"), 'ZIP installer missing'
    js("return (async()=>{const blob=await(await fetch(" + repr(path) + ")).blob();const data=new DataTransfer();data.items.add(new File([blob],'fixture.zip',{type:'application/zip'}));document.querySelector('.extension-install input').files=data.files;document.querySelector('.extension-install').requestSubmit()})()")
