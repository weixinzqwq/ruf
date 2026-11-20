(function () {
  if (window.__aoqiPayBridge) return;
  window.__aoqiPayBridge = true;
  function send(message) {
    const payload = JSON.stringify(message);
    if (window.chrome && window.chrome.webview) {
      window.chrome.webview.postMessage(payload);
    } else if (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.host) {
      window.webkit.messageHandlers.host.postMessage(payload);
    } else window.location.href = 'aoqi-host://popup/' + encodeURIComponent(payload);
  }
  window.payIframeObject = {
    closeDialog: function () { send({ type: 'close' }); },
    changeGoPayText: function () { send({ type: 'payHint' }); }
  };
  window.close = window.payIframeObject.closeDialog;
  window.addEventListener('message', function (event) {
    if (event.origin && event.origin !== window.location.origin) return;
    let data;
    try { data = typeof event.data === 'string' ? JSON.parse(event.data) : event.data; } catch (_) { return; }
    if (data && (data.action === 'closeDialog' || data.action === 'changeGoPayText')) window.payIframeObject[data.action]();
  });
  window.addEventListener('keydown', function (event) {
    if (event.key === 'Escape') { event.preventDefault(); window.payIframeObject.closeDialog(); }
  });
  function external(url) {
    try { url = new URL(String(url), window.location.href).href; } catch (_) { return; }
    if (/^(https?:\/\/|alipays?:\/\/|weixin:\/\/)/i.test(url)) send({ type: 'external', url: url });
  }
  window.open = function (url) { external(url); return null; };
  document.addEventListener('click', function (event) {
    const link = event.target.closest && event.target.closest('a[href]');
    if (link && (link.target === '_blank' || /^(alipays?|weixin):/i.test(link.href))) {
      event.preventDefault(); external(link.href);
    }
  }, true);
  function fit() { const width = window.innerWidth; if (document.documentElement && width > 0) document.documentElement.style.zoom = String(width / 626); }
  document.addEventListener('DOMContentLoaded', fit);
  window.addEventListener('resize', fit); fit(); send({ type: 'ready' });
})();
