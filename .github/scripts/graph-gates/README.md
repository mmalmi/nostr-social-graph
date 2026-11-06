# Isolated graph SDK gates

Install frozen JavaScript dependencies, the exact Rust toolchain, locked Cargo
sources and the pinned browser image before entering isolation. No package or
browser installation occurs during fixture execution.

`host.py` runs the existing TypeScript and Rust persistence suites, or builds
the production package, in a new Linux mount/network/PID namespace. It brings
up only loopback, mounts a private read-only sysfs view and drops to the runner's
non-root UID with no capabilities and no privilege escalation. The namespace
has an outer deadline and its PID namespace closes descendant processes.

Before loading fixtures, `network-proof.py` checks namespace identity, interfaces,
routes, UID and capabilities; verifies loopback TCP/UDP; and requires actual
IPv4/IPv6 TCP/UDP errors in both the process and a child. A timeout alone is not
accepted as denial.

`container.py` validates Docker's effective policy before starting the immutable
Playwright image: no external network, private PID/IPC namespaces, non-root UID,
no capabilities, read-only root filesystem, no swap and bounded memory/processes.
Its independent in-container deadline survives controller loss. The controller
collects logs and removes its exact container on normal completion or cancellation.

`browser.cjs` repeats the OS checks, then uses an unmodified Blob worker for
external HTTPS/WSS denial and a local WebSocket echo. Only after those unsigned
checks pass does it load the existing page and default-IIFE bootstrap/sync tests.
No routes, socket shims, dummy network interfaces or media permissions are used.

Production dist is built with Node 22.23.4 and pnpm 10.28.2. Browser validation
uses the separately recorded image runtime: Node 24.17.0, Playwright 1.61.1 and
Chromium 149.0.7827.55. The complete production dist hash manifest is checked
before and after browser validation. Gate receipts and full test logs are
uploaded even when a gate fails.
