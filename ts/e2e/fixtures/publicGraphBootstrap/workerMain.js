const worker = new Worker(new URL('./worker.js', import.meta.url));

const request = (data) => new Promise((resolve, reject) => {
  worker.onmessage = ({ data }) => data.error ? reject(new Error(data.error)) : resolve(data.result);
  worker.onerror = reject;
  worker.postMessage(data);
});

window.bootstrap = request;
window.syncRetained = request;
