(function () {
  if (window.__waterui) {
    return;
  }

  function toBase64Utf8(value) {
    return btoa(unescape(encodeURIComponent(value)));
  }

  function fromBase64Utf8(value) {
    return decodeURIComponent(escape(atob(value)));
  }

  window.__waterui = {
    pending: Object.create(null),
    toBase64Utf8: toBase64Utf8,
    fromBase64Utf8: fromBase64Utf8
  };

  window.__wateruiResolve = function (id, ok, payload) {
    var pending = window.__waterui.pending[id];
    if (!pending) {
      return;
    }
    delete window.__waterui.pending[id];
    if (ok) {
      pending.resolve(payload);
    } else {
      pending.reject(payload);
    }
  };
})();
