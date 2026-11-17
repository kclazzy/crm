// Встраивается приложением в каждую страницу. Работает только на страницах CRM,
// открытых через туннель (http://127.0.0.1:ПОРТ): добавляет «← Серверы».
(function () {
  if (location.protocol !== 'http:' || location.hostname !== '127.0.0.1') return;
  // Сигналы страницы CRM приложению (например, «пароль задан»).
  window.crmApp = { run: function (action) { location.href = 'http://crm-app.invalid/' + action; } };
  function add() {
    if (document.getElementById('crm-app-back')) return;
    var a = document.createElement('a');
    a.id = 'crm-app-back';
    a.href = '__APP_URL__#list';
    a.textContent = '← Серверы';
    a.setAttribute('style', 'position:fixed;left:8px;bottom:calc(8px + env(safe-area-inset-bottom));z-index:2147483647;'
      + 'padding:6px 10px;border-radius:999px;background:rgba(15,23,42,.72);color:#fff;'
      + 'font:500 12px/1 system-ui,sans-serif;text-decoration:none;opacity:.55');
    document.body.appendChild(a);
  }
  if (document.body) add(); else document.addEventListener('DOMContentLoaded', add);
  // Интерфейс CRM рассчитан на современный WebView. На телефоне, где системный WebView
  // давно не обновлялся, страница остаётся пустой — объясняем, что делать.
  setTimeout(function () {
    var root = document.getElementById('root');
    if (!root || root.childElementCount > 0 || document.getElementById('crm-app-old')) return;
    var box = document.createElement('div');
    box.id = 'crm-app-old';
    box.setAttribute('style', 'margin:48px 20px;font:16px/1.5 system-ui,sans-serif;color:#0f172a');
    box.innerHTML = '<h2 style="margin:0 0 8px">CRM не загрузилась</h2>'
      + '<p>Скорее всего, на телефоне устарел компонент «Android System WebView». '
      + 'Обновите его и Chrome в магазине приложений и откройте CRM снова.</p>'
      + '<p style="color:#475569">Если не помогло — нажмите «← Серверы» и «Проверить».</p>';
    document.body.appendChild(box);
  }, 8000);

  // Приложение было свёрнуто дольше пяти минут — возвращаемся на экран серверов:
  // он спросит отпечаток (если блокировка включена) и восстановит подключение.
  var hiddenAt = 0;
  document.addEventListener('visibilitychange', function () {
    if (document.hidden) { hiddenAt = Date.now(); return; }
    if (hiddenAt && Date.now() - hiddenAt > 5 * 60 * 1000) {
      location.href = '__APP_URL__#lock';
    }
  });
})();
