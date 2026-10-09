globalThis.__wateruiSend = function (envelope) {
  var request = JSON.parse(envelope);
  var resolve = globalThis.__wateruiResolve;
  globalThis.__wateruiNativeSend(envelope).then(
    function (replyText) {
      var reply = JSON.parse(replyText);
      resolve(request.id, reply.ok, reply.payload);
    },
    function (error) {
      resolve(request.id, false, { message: String(error) });
    },
  );
};
