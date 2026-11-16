// Экран «Мои серверы» приложения CRM для Android.
const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (id) => document.getElementById(id);

const LOCK_KEY = 'crm-lock';          // включена ли блокировка
const UNLOCKED = 'crm-unlocked';      // разблокировано в этом запуске (sessionStorage)
const STAY = location.hash === '#list'; // вернулись кнопкой «← Серверы» — не подключаться самим
// Приложение долго было свёрнуто — спросить отпечаток заново.
if (location.hash === '#lock') sessionStorage.removeItem(UNLOCKED);

function show(el, on) { el.hidden = !on; }
// Без replaceChildren: на телефонах со старым системным WebView (до 86) его нет.
function clear(el) { el.textContent = ''; }
function toast(text) { $('toast').textContent = text; show($('toast'), !!text); }

function describe(p) {
  if (p.state === 'online') {
    const how = p.node
      ? (p.relayed ? 'по ключу сервера, через узел связи' : p.via_internet ? 'по ключу сервера, напрямую' : 'в сети офиса')
      : p.via_internet ? 'через интернет' : 'в сети офиса';
    return `На связи — ${how}${p.latency_ms != null ? `, ${p.latency_ms} мс` : ''}`;
  }
  if (p.state === 'wrong_server') return 'Отвечает другой сервер — нужен новый ключ';
  return 'Не отвечает — нажмите «Проверить»';
}

// ---------- Блокировка отпечатком или PIN-кодом телефона ----------

async function biometricAvailable() {
  try { return !!(T.biometric && (await T.biometric.checkStatus()).isAvailable); } catch { return false; }
}

async function unlock() {
  show($('lock-error'), false);
  try {
    await T.biometric.authenticate('Вход в CRM', { allowDeviceCredential: true, cancelTitle: 'Отмена' });
    sessionStorage.setItem(UNLOCKED, '1');
    show($('lock'), false);
    start();
  } catch (e) {
    $('lock-error').textContent = 'Не получилось: ' + (e && e.message ? e.message : e);
    show($('lock-error'), true);
  }
}

// ---------- Серверы ----------

let expanded = null;

async function connect(id, invite) {
  toast('');
  try {
    const url = await invoke('server_open', { id, invite: invite || null });
    await invoke('open_window', { url });
  } catch (e) {
    toast(String(e));
    expanded = id;
    load();
  }
}

function checkPanel(s) {
  const box = document.createElement('div');
  const rows = document.createElement('ul');
  rows.className = 'checks';
  rows.innerHTML = '<li>Проверяю…</li>';
  const run = async () => {
    const list = await invoke('server_check', { id: s.id });
    clear(rows);
    for (const c of list) {
      const li = document.createElement('li');
      const dot = document.createElement('i');
      dot.className = `dot ${c.verdict === 'online' ? 'online' : c.verdict === 'wrong_server' ? 'wrong_server' : ''}`;
      const text = document.createElement('span');
      const b = document.createElement('b');
      b.textContent = c.addr;
      text.append(b, ` — ${c.message}`);
      li.append(dot, text);
      if (c.manual) {
        const x = document.createElement('button');
        x.textContent = 'Убрать';
        x.onclick = async () => { await invoke('server_remove_addr', { id: s.id, addr: c.addr }); run().catch((e) => toast(String(e))); };
        li.append(x);
      }
      rows.append(li);
    }
  };
  const form = document.createElement('form');
  form.className = 'addr';
  const input = document.createElement('input');
  input.placeholder = 'Адрес сервера';
  input.required = true;
  input.setAttribute('aria-label', 'Адрес сервера');
  input.autocapitalize = 'off';
  const add = document.createElement('button');
  add.textContent = 'Добавить';
  form.append(input, add);
  form.onsubmit = async (e) => {
    e.preventDefault();
    try { await invoke('server_add_addr', { id: s.id, addr: input.value }); input.value = ''; await run(); toast(''); }
    catch (x) { toast(String(x)); }
  };
  const del = document.createElement('button');
  del.className = 'danger';
  del.textContent = 'Удалить сервер из приложения';
  del.style.marginTop = '10px';
  let sure = false;
  del.onclick = async () => {
    if (!sure) { sure = true; del.textContent = 'Нажмите ещё раз, чтобы удалить'; return; }
    await invoke('server_remove', { id: s.id });
    expanded = null;
    load();
  };
  box.append(rows, form, del);
  run().catch((e) => { rows.innerHTML = ''; toast(String(e)); });
  return box;
}

async function load() {
  const list = await invoke('servers');
  const ul = $('servers');
  clear(ul);
  show($('empty'), list.length === 0);
  for (const s of list) {
    const li = document.createElement('li');
    const title = document.createElement('div');
    title.className = 'title';
    const dot = document.createElement('i');
    dot.className = `dot ${s.probe.state}`;
    title.append(dot, s.name);
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = describe(s.probe);
    const row = document.createElement('div');
    row.className = 'row';
    const open = document.createElement('button');
    open.className = 'primary';
    open.textContent = 'Подключиться';
    open.onclick = () => { open.disabled = true; open.textContent = 'Подключаюсь…'; connect(s.id).finally(() => { open.disabled = false; open.textContent = 'Подключиться'; }); };
    const check = document.createElement('button');
    check.textContent = 'Проверить';
    check.onclick = () => { expanded = expanded === s.id ? null : s.id; load(); };
    row.append(open, check);
    li.append(title, meta, row);
    if (expanded === s.id) li.append(checkPanel(s));
    ul.append(li);
  }
  return list;
}

// ---------- Добавление сервера ----------

async function pinNeeded() {
  const key = $('key').value.trim();
  if (!key) return false;
  try { return await invoke('key_needs_pin', { key }); } catch { return false; }
}

async function scan() {
  show($('add-error'), false);
  try {
    const s = T.barcodeScanner;
    if (!s) throw new Error('сканер доступен только на телефоне');
    let perm = await s.checkPermissions();
    if (perm !== 'granted') perm = await s.requestPermissions();
    if (perm !== 'granted') throw new Error('нет доступа к камере — разрешите его в настройках телефона');
    const r = await s.scan({ windowed: false, formats: [s.Format.QRCode] });
    $('key').value = r.content;
    show($('pin-row'), await pinNeeded());
  } catch (e) {
    $('add-error').textContent = 'Сканер: ' + (e && e.message ? e.message : e);
    show($('add-error'), true);
  }
}

$('add-open').onclick = () => { $('add-form').reset(); show($('pin-row'), false); show($('add-error'), false); $('add').showModal(); };
$('add-cancel').onclick = () => $('add').close();
$('scan').onclick = scan;
$('key').addEventListener('input', async () => show($('pin-row'), await pinNeeded()));
$('add-form').onsubmit = async (e) => {
  e.preventDefault();
  show($('add-error'), false);
  try {
    const added = await invoke('server_add', { key: $('key').value, pin: $('pin').value || null });
    $('add').close();
    await load();
    connect(added.id, added.invite);
  } catch (x) {
    $('add-error').textContent = String(x);
    show($('add-error'), true);
    show($('pin-row'), await pinNeeded());
  }
};

// ---------- Запуск ----------

async function start() {
  show($('app'), true);
  const list = await load();
  // Один сервер и он на связи — сразу открываем CRM, как обычное приложение.
  if (!STAY && list.length === 1 && list[0].probe.state === 'online') connect(list[0].id);
}

(async () => {
  const canLock = await biometricAvailable();
  show($('lock-setting'), canLock);
  $('lock-on').checked = localStorage.getItem(LOCK_KEY) === '1';
  $('lock-on').onchange = () => localStorage.setItem(LOCK_KEY, $('lock-on').checked ? '1' : '0');
  $('unlock').onclick = unlock;
  if (canLock && $('lock-on').checked && !sessionStorage.getItem(UNLOCKED)) {
    show($('lock'), true);
    unlock();
  } else {
    start().catch((e) => toast(String(e)));
  }
  // Вернулись в приложение — обновить статусы.
  document.addEventListener('visibilitychange', () => { if (!document.hidden && !$('app').hidden && !expanded) load().catch(() => {}); });
})();
