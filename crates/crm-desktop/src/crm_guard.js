// Встраивается программой CRM в каждую страницу окна. Работает только на страницах
// CRM, открытых через туннель (http://127.0.0.1:ПОРТ): добавляет «← Мои серверы»
// и, если интерфейс не загрузился, показывает причину вместо белого экрана.
(function () {
  if (location.protocol !== 'http:' || location.hostname !== '127.0.0.1') return;
  var APP = '__APP_URL__';
  var errors = [];
  window.addEventListener('error', function (e) {
    var t = e.target;
    if (t && t !== window && (t.src || t.href)) {
      errors.push('Не загрузился файл ' + String(t.src || t.href).replace(location.origin, ''));
      return;
    }
    errors.push((e.message || 'ошибка') + (e.filename ? ' — ' + e.filename.replace(location.origin, '') + ':' + e.lineno : ''));
  }, true);
  window.addEventListener('unhandledrejection', function (e) {
    errors.push('Promise: ' + (e.reason && e.reason.message ? e.reason.message : String(e.reason)));
  });
  // Кнопки страницы, которым нужна программа: window.crmApp.run('vpn-apply').
  window.crmApp = { run: function (action) { location.href = 'http://__APP_HOST__/' + action; } };
  // Назад в программу: у владельца — панель сервера, у сотрудника — «Мои серверы».
  function back() { location.href = APP; }
  function el(tag, css, text) {
    var n = document.createElement(tag);
    n.style.cssText = css;
    if (text) n.textContent = text;
    return n;
  }
  var btnCss = 'font:600 13px Segoe UI,system-ui,sans-serif;border-radius:999px;padding:7px 14px;cursor:pointer;';
  function addBack() {
    if (document.getElementById('crm-app-back') || !document.body) return;
    var b = el('button', btnCss + 'position:fixed;left:12px;bottom:12px;z-index:2147483647;border:1px solid #cbd5e1;background:rgba(255,255,255,.92);color:#334155;box-shadow:0 2px 8px rgba(0,0,0,.12);', '← В программу CRM');
    b.id = 'crm-app-back';
    b.title = 'Панель управления или список серверов';
    b.onclick = back;
    document.body.appendChild(b);
  }
  function blank() {
    // Только страница CRM (у неё есть скрипт интерфейса), а не сообщение туннеля «сервер не отвечает».
    if (!document.querySelector('script[type="module"]')) return false;
    var root = document.getElementById('root');
    return !root || root.childElementCount === 0;
  }
  function explain() {
    if (!blank() || document.getElementById('crm-app-blank')) return;
    var box = el('div', 'position:fixed;inset:0;z-index:2147483646;background:#f8fafc;color:#0f172a;display:flex;align-items:center;justify-content:center;font:15px/1.5 Segoe UI,system-ui,sans-serif;padding:24px;');
    box.id = 'crm-app-blank';
    var card = el('div', 'max-width:560px;background:#fff;border:1px solid #e2e8f0;border-radius:16px;padding:28px;box-shadow:0 8px 30px rgba(15,23,42,.08);');
    card.appendChild(el('h2', 'margin:0 0 8px;font-size:20px;', 'CRM не загрузилась'));
    card.appendChild(el('p', 'margin:0 0 12px;color:#475569;', 'Сервер ответил, но интерфейс не запустился. Нажмите «Обновить». Если не помогает — пришлите снимок этого окна владельцу или разработчику.'));
    var info = 'Адрес: ' + location.href + '\nБраузер окна: ' + navigator.userAgent + (errors.length ? '\nОшибки:\n' + errors.slice(0, 6).join('\n') : '\nОшибок скриптов нет — возможно, страница ещё не пришла с сервера.');
    card.appendChild(el('pre', 'white-space:pre-wrap;word-break:break-all;background:#f1f5f9;border-radius:10px;padding:10px;font:12px/1.45 Consolas,monospace;max-height:220px;overflow:auto;user-select:text;', info));
    var row = el('div', 'display:flex;gap:8px;margin-top:16px;flex-wrap:wrap;');
    var r = el('button', btnCss + 'border:none;background:#4f46e5;color:#fff;', 'Обновить');
    r.onclick = function () { location.reload(); };
    var b = el('button', btnCss + 'border:1px solid #cbd5e1;background:#fff;color:#334155;', '← В программу CRM');
    b.onclick = back;
    row.appendChild(r);
    row.appendChild(b);
    card.appendChild(row);
    box.appendChild(card);
    document.body.appendChild(box);
  }
  function report() {
    var root = document.getElementById('root');
    var d = JSON.stringify({
      url: location.href,
      title: document.title,
      ready: document.readyState,
      root_children: root ? root.childElementCount : -1,
      text: (document.body ? document.body.innerText : '').slice(0, 160),
      errors: errors.slice(0, 8),
      ua: navigator.userAgent,
    });
    location.href = 'http://__REPORT_HOST__/?d=' + encodeURIComponent(d);
  }
  function ready() {
    addBack();
    setTimeout(explain, 7000);
    if (__SELFTEST__) {
      // Скачивание файла из памяти страницы (как «Скачать конфиг» VPN) и вызов программы.
      setTimeout(function () {
        var a = document.createElement('a');
        a.href = URL.createObjectURL(new Blob(['CRM selftest'], { type: 'text/plain' }));
        a.download = 'crm-selftest.txt';
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(function () { window.crmApp.run('ping'); }, 500);
      }, 4000);
      setTimeout(report, 8000);
    }
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', ready);
  else ready();
  // Страница могла смениться без перезагрузки (React) — кнопка остаётся.
  setInterval(addBack, 2000);
})();
