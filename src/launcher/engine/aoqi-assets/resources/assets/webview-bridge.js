(function () {
  if (window.__aoqiHostBridge) { send({ type: 'ready' }); return; }
  window.__aoqiHostBridge = true;
  function send(value) {
    const payload = JSON.stringify(value);
    if (window.chrome && window.chrome.webview) window.chrome.webview.postMessage(payload);
    else window.webkit.messageHandlers.host.postMessage(payload);
  }
  function snapshot() {
    const values = {};
    ['getDeviceInfo', 'getYidunLoginToken', 'getYidunResiterToken', 'getCookieId', 'getSessionId'].forEach(function (name) {
      try { if (typeof window[name] === 'function') values[name] = window[name](); } catch (_) {}
    });
    send({ type: 'snapshot', cookies: document.cookie, values: values });
  }
  const movie = new Proxy({}, {
    get: function (_, name) {
      return function () {
        send({ type: 'callback', name: String(name), args: Array.prototype.slice.call(arguments) });
      };
    }
  });
  window.getFlash = function () { return movie; };
  // The official game page's published DOM interface is named myFlash.
  try { Object.defineProperty(window, 'myFlash', { configurable: true, value: movie }); } catch (_) {}
  const legacy = document.createElement('form');
  legacy.name = 'myFlash';
  legacy.style.display = 'none';
  ['callASFromJS','getVerURLRequestForJs','getAntiAddictionStateForJs','getQiQiStatus'].forEach(function (name) {
    legacy[name] = movie[name];
  });
  document.body.appendChild(legacy);
  window.callAS = function () {
    movie.callASFromJS.apply(null, arguments);
  };
  window.logout = function () { send({ type: 'reload' }); };
  window.showPayDialog = function (data) { send({ type: 'payDialog', data: data }); };
  ['payObject','payIframeObject'].forEach(function (name) {
    if (window[name] && typeof window[name].showPayDialog === 'function') window[name].showPayDialog = window.showPayDialog;
  });
  function closePage() { send({ type: 'close' }); }
  window.close = closePage;
  window.closeUrlInGame = closePage;
  window.hideIFrame = closePage;
  window.addEventListener('keydown', function (event) {
    if (event.key === 'Escape') { event.preventDefault(); closePage(); }
  });
  function receive(event) {
    let request;
    try { request = typeof event.data === 'string' ? JSON.parse(event.data) : event.data; } catch (_) { return; }
    if (!request || request.type !== 'invoke' || typeof request.name !== 'string') return;
    try {
      let result;
      if (request.name === 'eval') result = (0, eval)(String(request.args[0]));
      else if (typeof window[request.name] === 'function') result = window[request.name].apply(window, request.args || []);
      send({ type: 'result', name: request.name, value: result === undefined ? null : result });
    } catch (_) { send({ type: 'unsupported', name: request.name }); }
    snapshot();
  }
  window.addEventListener('message', receive);
  if (window.chrome && window.chrome.webview && typeof window.chrome.webview.addEventListener === 'function') {
    window.chrome.webview.addEventListener('message', receive);
  }
  // The page supplies its own login and verification code; this adapter only transports messages.
  document.querySelectorAll('object,embed').forEach(function (element) { element.remove(); });
  const help = document.getElementById('browserInspectPopDiv');
  if (help) help.remove();
  snapshot();
  window.setInterval(snapshot, 1000);
  send({ type: 'ready' });
})();
