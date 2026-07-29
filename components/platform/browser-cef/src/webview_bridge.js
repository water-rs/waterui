(() => {
    const pending = new Map();
    let nextId = 0;
    globalThis.__wateruiResolve = (id, data) => {
        const resolve = pending.get(id);
        if (resolve === undefined) {
            throw new Error(`Unknown WaterUI bridge request ${id}`);
        }
        pending.delete(id);
        resolve(Uint8Array.from(data));
    };
    globalThis.waterui = Object.freeze({
        invoke(name, data = []) {
            const id = ++nextId;
            const bytes = Array.from(data);
            return new Promise((resolve) => {
                pending.set(id, resolve);
                globalThis.__wateruiInvoke(JSON.stringify({ id, name, data: bytes }));
            });
        },
    });
})();
