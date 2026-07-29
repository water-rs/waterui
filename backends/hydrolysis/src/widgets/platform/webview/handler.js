(function () {
  var name = __WATERUI_HANDLER__;
  if (!window.__waterui || !window.__wateruiResolve) {
    return;
  }
  if (window[name] && window[name].__wateruiWrapped) {
    return;
  }

  function send(data) {
    var id = String(Date.now()) + "_" + String(Math.random()).slice(2);
    var text = typeof data === "string" ? data : JSON.stringify(data);
    var payload = window.__waterui.toBase64Utf8(text);
    return new Promise(function (resolve, reject) {
      window.__waterui.pending[id] = {
        resolve: resolve,
        reject: reject
      };
      window.webkit.messageHandlers[name].postMessage(id + ":" + payload);
    });
  }

  window[name] = {
    __wateruiWrapped: true,
    postMessageRaw: function (data) {
      return send(data);
    },
    postMessage: function (data) {
      return send(data).then(function (reply) {
        return window.__waterui.fromBase64Utf8(reply);
      });
    }
  };
})();
