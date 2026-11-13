// Окно CRM: показывает экран по состоянию сервера, которое присылает приложение.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const screens = ['loading', 'welcome', 'owner', 'dashboard', 'down', 'nolink'];
let busy = false;

function show(id) {
  for (const s of screens) $(s).hidden = s !== id;
}

function toast(text) {
  const t = $('toast');
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => { t.hidden = true; }, 5000);
}

function render(st) {
  const pill = $('pill');
  pill.className = `pill ${st.light}`;
  pill.querySelector('span').textContent = st.text;
  if (busy) return;

  if (st.reachable && st.needs_owner) return show('owner');
  if (st.reachable) {
    $('fingerprint').textContent = st.ca_fingerprint || '— (HTTPS выключен)';
    show('dashboard');
    loadAddresses();
    return;
  }
  if (st.service === 'not_installed') return show('welcome');
  if (st.service === 'stopped' || st.service === 'running') {
    $('down-title').textContent = st.text;
    return show('down');
  }
  show('nolink');
}

async function loadAddresses() {
  const list = await invoke('addresses');
  const ul = $('addresses');
  ul.replaceChildren();
  if (list.length === 0) {
    const li = document.createElement('li');
    li.textContent = 'Нет подключения к сети';
    ul.append(li);
  }
  for (const a of list) {
    const li = document.createElement('li');
    const span = document.createElement('span');
    span.textContent = a;
    const copy = document.createElement('button');
    copy.textContent = 'Копировать';
    copy.onclick = async () => {
      await navigator.clipboard.writeText(a);
      copy.textContent = 'Скопировано';
      setTimeout(() => { copy.textContent = 'Копировать'; }, 1500);
    };
    li.append(span, copy);
    ul.append(li);
  }
}

/** Выполнить действие со службой: кнопка показывает загрузку, ошибка — во всплывашке. */
async function act(button, action) {
  busy = true;
  button.classList.add('busy');
  button.disabled = true;
  try {
    render(await invoke('server_action', { action }));
  } catch (e) {
    toast(String(e));
  } finally {
    busy = false;
    button.classList.remove('busy');
    button.disabled = false;
    render(await invoke('get_status'));
  }
}

$('install').onclick = (e) => act(e.currentTarget, 'install');
$('start').onclick = (e) => act(e.currentTarget, 'start');
$('restart').onclick = (e) => act(e.currentTarget, 'restart');
$('stop').onclick = () => $('confirm-stop').showModal();
$('stop-cancel').onclick = () => $('confirm-stop').close();
$('stop-ok').onclick = () => {
  $('confirm-stop').close();
  act($('stop'), 'stop');
};

$('owner-form').onsubmit = async (e) => {
  e.preventDefault();
  const f = new FormData(e.currentTarget);
  const err = $('owner-error');
  err.hidden = true;
  if (f.get('password') !== f.get('password2')) {
    err.textContent = 'Пароли не совпадают';
    err.hidden = false;
    return;
  }
  const button = e.currentTarget.querySelector('button');
  button.classList.add('busy');
  button.disabled = true;
  try {
    render(await invoke('create_owner', {
      username: f.get('username'),
      displayName: f.get('display_name'),
      password: f.get('password'),
    }));
    e.currentTarget.reset();
    toast('Владелец создан. Добро пожаловать!');
  } catch (x) {
    err.textContent = String(x);
    err.hidden = false;
  } finally {
    button.classList.remove('busy');
    button.disabled = false;
  }
};

listen('status', (e) => render(e.payload));
listen('confirm-stop', () => $('confirm-stop').showModal());
invoke('get_status').then(render).catch((e) => toast(String(e)));
