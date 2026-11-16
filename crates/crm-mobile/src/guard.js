// Встраивается приложением в каждую страницу. Работает только на страницах CRM,
// открытых через туннель (http://127.0.0.1:ПОРТ): добавляет «← Серверы».
(function () {
  if (location.protocol !== 'http:' || location.hostname !== '127.0.0.1') return;
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
