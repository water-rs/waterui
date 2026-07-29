(() => {
    const pending = new Map();
    let nextId = 1;

    const decode = value => Uint8Array.from(atob(value), character => character.charCodeAt(0));
    globalThis.__wateruiResolve = (id, value) => {
        const resolve = pending.get(id);
        if (resolve === undefined) {
            throw new Error(`Unknown WaterUI message id ${id}`);
        }
        pending.delete(id);
        resolve(decode(value));
    };

    globalThis.__wateruiInstallHandler = name => {
        if (globalThis.webkit.messageHandlers[name] !== undefined) {
            throw new Error(`WaterUI handler ${name} is already installed`);
        }
        globalThis.webkit.messageHandlers[name] = {
            postMessage(data) {
                const id = nextId++;
                const bytes = Array.from(data instanceof Uint8Array ? data : new Uint8Array(data));
                const result = new Promise(resolve => pending.set(id, resolve));
                globalThis.webkit.messageHandlers.__waterui.postMessage(JSON.stringify({
                    id,
                    name,
                    data: bytes,
                }));
                return result;
            },
        };
    };

    globalThis.__wateruiRemoveHandler = name => {
        if (globalThis.webkit.messageHandlers[name] === undefined) {
            throw new Error(`WaterUI handler ${name} is not installed`);
        }
        delete globalThis.webkit.messageHandlers[name];
    };
})();
