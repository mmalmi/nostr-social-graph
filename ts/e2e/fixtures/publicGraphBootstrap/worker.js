import { bootstrap, syncRetained } from './bootstrap.js';

self.onmessage = async ({ data }) => {
  try {
    self.postMessage({ result: typeof data === 'string' ? await bootstrap(data) : await syncRetained(data) });
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};
