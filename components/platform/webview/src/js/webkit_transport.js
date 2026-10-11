// Adapts the shared bridge's one-function `__wateruiSend` transport onto
// WebKit's reply-capable script message channel: `postMessage` returns a
// promise that resolves with the reply envelope the native handler produced.
// The reply is bound to the document that sent the message; if that document
// has gone, the engine drops the reply instead of answering its successor.
globalThis.__wateruiSend = function (envelope) {
  var request = JSON.parse(envelope);
  var resolve = globalThis.__wateruiResolve;
  window.webkit.messageHandlers.__wateruiSend.postMessage(envelope).then(
    function (replyText) {
      var reply = JSON.parse(replyText);
      resolve(request.id, reply.ok, reply.payload);
    },
    function (error) {
      resolve(request.id, false, { message: String(error) });
    },
  );
};
