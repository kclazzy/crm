// Окно CRM: показывает экран по состоянию сервера, которое присылает приложение.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const screens = ['loading', 'welcome', 'owner', 'dashboard', 'down', 'nolink', 'servers'];
// Экран списка серверов открыт явно (из трея или кнопкой) — состояние сервера его не перебивает.
const query = new URLSearchParams(location.search);
let pinned = query.get('screen') === 'servers';
// Экран выбран пользователем (кнопкой «Назад» или «Подключиться») — состояние сервера его не перебивает.
let chosen = null;
let lastStatus = null;
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
  lastStatus = st;
  const pill = $('pill');
  pill.className = `pill ${st.light}`;
  pill.querySelector('span').textContent = st.text;
  if (busy || pinned || chosen) return;

  if (st.reachable && st.needs_owner) return show('owner');
  if (st.reachable) {
    $('fingerprint').textContent = st.ca_fingerprint || '— (HTTPS выключен)';
    show('dashboard');
    loadAddresses();
    return;
  }
  if (st.service === 'not_installed') return knownServers.length ? openServers() : show('welcome');
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
$('install-fresh').onclick = (e) => act(e.currentTarget, 'install-fresh');
// Прежний сервер на этом компьютере: «Установить сервер» продолжит с его базой,
// «Начать с чистой базы» отложит её в сторону.
invoke('old_server_data')
  .then((path) => {
    if (!path) return;
    $('old-data-path').textContent = path;
    $('install').textContent = 'Продолжить с прежней базой';
    $('old-data').hidden = false;
  })
  .catch(() => {});
$('start').onclick = (e) => act(e.currentTarget, 'start');
$('restart').onclick = (e) => act(e.currentTarget, 'restart');
$('stop').onclick = () => $('confirm-stop').showModal();
$('stop-cancel').onclick = () => $('confirm-stop').close();
$('remove-server').onclick = () => $('confirm-remove').showModal();
$('remove-cancel').onclick = () => $('confirm-remove').close();
$('remove-ok').onclick = async () => {
  $('confirm-remove').close();
  await act($('remove-server'), 'uninstall');
  const path = await invoke('old_server_data').catch(() => null);
  if (path) {
    $('old-data-path').textContent = path;
    $('install').textContent = 'Продолжить с прежней базой';
    $('old-data').hidden = false;
  }
};
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

// ---------- Мои серверы ----------

let knownServers = [];
// Форма добавления раскрыта кнопкой «+ Добавить сервер».
let adding = false;

// «Где вы сейчас?»: в офисе — адрес и код, не в офисе — ключ и PIN. Выбор запоминается.
function setWhere(where) {
  document.querySelectorAll('.segmented [data-where]').forEach((b) => {
    b.setAttribute('aria-checked', String(b.dataset.where === where));
  });
  $('enroll-form').hidden = where !== 'office';
  $('add-form').hidden = where !== 'away';
  try { localStorage.setItem('crm.where', where); } catch { /* без памяти */ }
}
document.querySelectorAll('.segmented [data-where]').forEach((b) => {
  b.onclick = () => setWhere(b.dataset.where);
});
try { setWhere(localStorage.getItem('crm.where') === 'away' ? 'away' : 'office'); } catch { setWhere('office'); }
$('add-more').onclick = () => {
  adding = true;
  $('add-more').hidden = true;
  $('add-card').hidden = false;
};
// Вставили ключ CRM1-… в поле кода — значит, это подключение не из офиса.
document.querySelector('#enroll-form [name=code]').addEventListener('paste', (e) => {
  const text = (e.clipboardData && e.clipboardData.getData('text')) || '';
  if (!/^\s*CRM1-/i.test(text)) return;
  e.preventDefault();
  setWhere('away');
  keyInput.value = text.trim();
  keyInput.dispatchEvent(new Event('input'));
});

function describe(p) {
  if (p.state === 'online') {
    const how = p.node
      ? (p.relayed ? 'по ключу сервера, через узел связи' : p.via_internet ? 'по ключу сервера, напрямую' : 'в сети офиса')
      : p.via_internet ? 'через интернет' : 'в сети офиса';
    return `В сети · ${how} · ${p.latency_ms} мс`;
  }
  if (p.state === 'wrong_server') return 'Чужой сертификат — подключение запрещено';
  return 'Не отвечает — нажмите «Проверить»';
}

async function loadServers() {
  knownServers = await invoke('servers');
  const ul = $('server-list');
  ul.replaceChildren();
  // Серверы уже есть — форма добавления свёрнута под «+ Добавить сервер».
  const has = knownServers.length > 0;
  $('add-more').hidden = !has || adding;
  $('add-card').hidden = has && !adding;
  for (const s of knownServers) {
    const li = document.createElement('li');
    const dot = document.createElement('i');
    dot.className = `dot ${s.probe.state}`;
    const info = document.createElement('div');
    info.className = 'info';
    const name = document.createElement('div');
    name.className = 'name';
    name.textContent = s.user ? `${s.name} · ${s.user}` : s.name;
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = describe(s.probe);
    info.append(name, meta);
    const actions = document.createElement('div');
    actions.className = 'row-actions';
    const open = document.createElement('button');
    open.className = 'primary';
    open.textContent = 'Подключиться';
    open.disabled = s.probe.state === 'wrong_server';
    open.onclick = () => connect(open, s.id, null);
    const del = document.createElement('button');
    del.textContent = 'Убрать';
    del.onclick = async () => {
      if (!confirm(`Убрать «${s.name}» из списка? Данные на сервере не пострадают.`)) return;
      await invoke('server_remove', { id: s.id });
      loadServers();
    };
    const check = document.createElement('button');
    check.textContent = 'Проверить';
    check.title = 'Проверить соединение и адреса сервера';
    check.onclick = () => {
      expanded = expanded === s.id ? null : s.id;
      loadServers();
    };
    actions.append(open, check, del);
    li.append(dot, info, actions);
    if (expanded === s.id) li.append(checkPanel(s));
    ul.append(li);
  }
}

// ---------- Проверка соединения и адреса сервера ----------

// Сервер, у которого раскрыта проверка (пока раскрыта — список сам не перерисовывается).
let expanded = null;

function checkPanel(s) {
  const box = document.createElement('div');
  box.className = 'check-panel';
  const rows = document.createElement('ul');
  rows.className = 'checks';
  const wait = document.createElement('li');
  wait.textContent = 'Проверяю адреса…';
  rows.append(wait);

  const run = async () => {
    const list = await invoke('server_check', { id: s.id });
    rows.replaceChildren();
    for (const c of list) {
      const li = document.createElement('li');
      const dot = document.createElement('i');
      dot.className = `dot ${c.verdict === 'online' ? 'online' : c.verdict === 'wrong_server' ? 'wrong_server' : ''}`;
      const text = document.createElement('span');
      const addr = document.createElement('b');
      addr.textContent = c.addr;
      text.append(addr, ` — ${c.message}`);
      li.append(dot, text);
      if (c.manual) {
        const x = document.createElement('button');
        x.textContent = 'Убрать';
        x.onclick = async () => {
          await invoke('server_remove_addr', { id: s.id, addr: c.addr });
          run().catch((e) => toast(String(e)));
        };
        li.append(x);
      }
      rows.append(li);
    }
    if (!list.length) {
      const li = document.createElement('li');
      li.textContent = 'У сервера нет адресов — добавьте адрес ниже.';
      rows.append(li);
    }
  };

  const form = document.createElement('form');
  form.className = 'addr-form';
  const input = document.createElement('input');
  input.placeholder = 'Адрес сервера: 192.168.1.10 или office.example.ru';
  input.required = true;
  input.spellcheck = false;
  input.setAttribute('aria-label', 'Адрес сервера');
  const add = document.createElement('button');
  add.type = 'submit';
  add.textContent = 'Добавить адрес';
  const again = document.createElement('button');
  again.type = 'button';
  again.textContent = 'Проверить ещё раз';
  again.onclick = () => run().catch((e) => toast(String(e)));
  const err = document.createElement('p');
  err.className = 'error';
  err.hidden = true;
  form.append(input, add, again);
  form.onsubmit = async (e) => {
    e.preventDefault();
    err.hidden = true;
    try {
      await invoke('server_add_addr', { id: s.id, addr: input.value });
      input.value = '';
      await run();
    } catch (x) {
      err.textContent = String(x);
      err.hidden = false;
    }
  };
  const hint = document.createElement('small');
  hint.textContent = 'Адрес можно ввести самому, если сервер переехал или сменился IP. Чужой сервер по неверному адресу не подключится: программа сверяет сертификат из ключа.';
  box.append(rows, form, err, hint);
  run().catch((e) => { wait.textContent = String(e); });
  return box;
}

async function connect(button, id, invite) {
  button.classList.add('busy');
  button.disabled = true;
  try {
    await invoke('server_open', { id, invite });
  } catch (e) {
    toast(String(e));
    expanded = id; // не подключились — сразу показать, какой адрес почему не отвечает
    loadServers();
  } finally {
    button.classList.remove('busy');
    button.disabled = false;
  }
}

function openServers() {
  pinned = true;
  chosen = null;
  show('servers');
  loadServers().catch((e) => toast(String(e)));
}

$('to-servers').onclick = openServers;
$('dash-servers').onclick = openServers;
$('enter-owner').onclick = async (e) => {
  const b = e.currentTarget;
  b.classList.add('busy');
  b.disabled = true;
  try { await invoke('enter_as_owner'); } catch (x) { toast(String(x)); } finally {
    b.classList.remove('busy');
    b.disabled = false;
  }
};
$('open-own').onclick = async (e) => {
  const b = e.currentTarget;
  b.classList.add('busy');
  try { await invoke('open_own'); } catch (x) { toast(String(x)); } finally { b.classList.remove('busy'); }
};

const keyInput = document.querySelector('#add-form textarea');
keyInput.addEventListener('input', async () => {
  try {
    $('pin-row').hidden = !(await invoke('key_needs_pin', { key: keyInput.value }));
  } catch { $('pin-row').hidden = true; }
});

// Сервер добавлен (по ключу или по коду): сразу открыть — «Задайте пароль» для нового
// сотрудника; не отвечает — показать, какой адрес почему.
async function openAdded(added) {
  adding = false;
  await loadServers();
  if (added.probe.state === 'online') {
    await invoke('server_open', { id: added.id, invite: added.invite });
  } else {
    toast(`Сервер «${added.name}» добавлен, но сейчас ${describe(added.probe).toLowerCase()}`);
    expanded = added.id;
    loadServers();
  }
}

$('enroll-form').onsubmit = async (e) => {
  e.preventDefault();
  const form = e.currentTarget;
  const f = new FormData(form);
  const err = $('enroll-error');
  err.hidden = true;
  const button = form.querySelector('button');
  button.classList.add('busy');
  button.disabled = true;
  try {
    const added = await invoke('server_enroll', { addr: f.get('addr'), code: f.get('code') });
    form.reset();
    await openAdded(added);
  } catch (x) {
    err.textContent = String(x);
    err.hidden = false;
  } finally {
    button.classList.remove('busy');
    button.disabled = false;
  }
};

$('add-form').onsubmit = async (e) => {
  e.preventDefault();
  const form = e.currentTarget;
  const f = new FormData(form);
  const err = $('add-error');
  err.hidden = true;
  const button = form.querySelector('button');
  button.classList.add('busy');
  button.disabled = true;
  try {
    const added = await invoke('server_add', { key: f.get('key'), pin: f.get('pin') || null });
    form.reset();
    $('pin-row').hidden = true;
    await openAdded(added);
  } catch (x) {
    err.textContent = String(x);
    err.hidden = false;
  } finally {
    button.classList.remove('busy');
    button.disabled = false;
  }
};

listen('toast', (e) => toast(String(e.payload)));
listen('status', (e) => render(e.payload));
listen('confirm-stop', () => $('confirm-stop').showModal());
if (pinned) openServers();
invoke('servers')
  .then((list) => { knownServers = list; })
  .catch(() => {})
  .finally(() => invoke('get_status').then(render).catch((e) => toast(String(e))));

// Статус серверов обновляется, пока список на экране.
setInterval(() => {
  if (!$('servers').hidden && !expanded) loadServers().catch(() => {});
}, 10000);

// «Назад» на каждом экране: к выбору «сервер или подключение»; у владельца сервера — к панели.
document.querySelectorAll('[data-back]').forEach((b) => {
  b.addEventListener('click', () => {
    pinned = false;
    const host = lastStatus && lastStatus.reachable && !lastStatus.needs_owner;
    chosen = host ? null : 'welcome';
    if (host) render(lastStatus);
    else show('welcome');
  });
});
// Выбор на приветственном экране снимает «Назад».
$('install').addEventListener('click', () => { chosen = null; }, true);
$('install-fresh').addEventListener('click', () => { chosen = null; }, true);

// ---------- Автозапуск ----------
async function loadAutostart() {
  try {
    const a = await invoke('get_autostart');
    $('as-server').checked = !!a.server;
    $('as-server').disabled = a.server === null;
    $('as-app').checked = a.app;
  } catch { /* настройки недоступны — переключатели останутся как есть */ }
}
$('as-server').addEventListener('change', async (e) => {
  const box = e.currentTarget;
  box.disabled = true;
  try {
    await invoke('set_server_autostart', { on: box.checked });
    toast(box.checked ? 'Сервер будет запускаться вместе с Windows' : 'Сервер больше не запускается сам');
  } catch (x) {
    toast(String(x));
  } finally {
    await loadAutostart();
  }
});
$('as-app').addEventListener('change', async (e) => {
  try { await invoke('set_app_autostart', { on: e.currentTarget.checked }); } catch (x) { toast(String(x)); }
  loadAutostart();
});
loadAutostart();

// Пришли из трея: показать подтверждение или итог действия.
if (query.get('toast')) setTimeout(() => toast(query.get('toast')), 300);
if (query.get('confirm') === 'stop') setTimeout(() => $('confirm-stop').showModal(), 300);
if (query.has('toast') || query.has('confirm')) history.replaceState(null, '', location.pathname);
