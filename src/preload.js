const listeners = {};

globalThis.ipcRenderer = {
    on: (eventName, handler) => {
        if(!listeners[eventName]) listeners[eventName] = []
        listeners[eventName].push(handler)
    },
    emit: (eventName, payload) => {
        const message = { channel: eventName, payload }
        globalThis.ipc.postMessage(JSON.stringify(message))
    }
}

globalThis.__denoEmit = (event) => {
    const { channel, payload } = event;
    (listeners[channel] || []).forEach(fn => fn(JSON.parse(payload)));
}