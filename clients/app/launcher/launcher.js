const invoke = window.__TAURI__.core.invoke;
const form = document.querySelector('#launch-form');
const workspace = document.querySelector('#workspace');
const config = document.querySelector('#config');
const start = document.querySelector('#start');
const choose = document.querySelector('#choose-folder');
const status = document.querySelector('#status');
const errorBox = document.querySelector('#error-box');
let busy = false;

function showError(error) {
  document.querySelector('#error').textContent = String(error);
  errorBox.hidden = false;
}
function setBusy(value) {
  busy = value;
  for (const element of [workspace, config, start, choose]) element.disabled = value;
  status.textContent = value ? 'Запускаю агента…' : '';
  start.textContent = value ? 'Открываю…' : 'Открыть проект ↗';
}
async function launch() {
  if (busy || !form.reportValidity()) return;
  errorBox.hidden = true;
  setBusy(true);
  try {
    await invoke('start_agent', { preferences: { workspace: workspace.value.trim(), config: config.value.trim() } });
    status.textContent = 'Проект открыт';
  } catch (error) { showError(error); }
  finally { setBusy(false); }
}
form.addEventListener('submit', event => { event.preventDefault(); launch(); });
choose.addEventListener('click', async () => {
  choose.disabled = true;
  try { const selected = await invoke('choose_workspace'); if (selected) workspace.value = selected; }
  catch (error) { showError(error); }
  finally { choose.disabled = false; }
});
async function initialize() {
  try {
    const state = await invoke('launcher_state');
    workspace.value = state.preferences.workspace;
    config.value = state.preferences.config;
    for (const profile of state.profiles) {
      const option = document.createElement('option'); option.value = profile;
      document.querySelector('#profiles').append(option);
    }
    document.querySelector('#switch-note').hidden = !state.running;
    setBusy(false);
    if (state.error) showError(state.error);
    if (state.auto_start) await launch();
  } catch (error) { setBusy(false); showError(error); }
}
initialize();
