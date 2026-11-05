import { bootstrap } from './bootstrap.js';

self.onmessage = async ({ data: url }) => {
  try {
    self.postMessage({ result: await bootstrap(url) });
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};
