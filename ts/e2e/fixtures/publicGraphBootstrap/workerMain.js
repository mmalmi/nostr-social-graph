const worker = new Worker(new URL('./worker.js', import.meta.url));

window.bootstrap = (url) => new Promise((resolve, reject) => {
  worker.onmessage = ({ data }) => data.error ? reject(new Error(data.error)) : resolve(data.result);
  worker.onerror = reject;
  worker.postMessage(url);
});
