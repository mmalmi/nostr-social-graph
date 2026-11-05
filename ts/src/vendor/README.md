# Negentropy reference implementation

`negentropy.js` is the MIT-licensed reference implementation by Doug Hoyte,
from https://github.com/hoytech/negentropy at
`fe0bb8ad4d636776e2e16c4f2369c4eae47f1b34`, path `js/Negentropy.js`.
Original SHA-256: `0ccda2fb5343f97a0679442aec55e258cab09ddc775a3f8af87ada7b3c02cb50`.
Local changes: ESM exports and platform Web Crypto instead of CommonJS crypto.
Only the opt-in publicGraphSync entry imports this implementation.
