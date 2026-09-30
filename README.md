# P6 Vault

A desktop patch manager for the Sequential **Prophet-6**: import old `.syx` archives, audition sounds on the synth's edit buffer, categorize and organize them in groups, build a complete 500-slot **New** bank, compare it with **Current** (A/B), and write only the changed user slots. Every write is preceded by a verified full backup, checked by reading each slot back, and recoverable if it is interrupted.

- **Native app:** Tauri 2 + React/TypeScript. All protocol, bank-operation and write-permission logic is in Rust (`crates/p6-core`).
- **Offline first:** importing, browsing, categorizing, organizing, undo and export never send MIDI. You only need the synth to read it, audition and write.
- **Simulator mode:** a simulated Prophet-6 with its own separate library, so you can try every workflow (including faults and recovery) without hardware.

> Status: the software is complete and tested against the simulator (unit, integration and end-to-end tests). **It has not yet been tested with a real Prophet-6.** Until you complete the single-slot test in [docs/HARDWARE-TESTS.md](docs/HARDWARE-TESTS.md), writes to real hardware are limited to one changed slot. See [docs/STATUS.md](docs/STATUS.md).

## Build and launch on your Mac

Requirements: macOS 11+, Xcode Command Line Tools (`xcode-select --install`), Rust (https://rustup.rs; the pinned toolchain in `rust-toolchain.toml` installs itself), Node.js 20+ and pnpm (`npm i -g pnpm`).

```bash
git clone https://github.com/arenttodal/P6-VAULT.git && cd P6-VAULT
git checkout claude/optimistic-mayer-9twvi9
./scripts/build-macos.sh            # runs all checks, then builds the release app + dmg
open "target/release/bundle/macos/P6 Vault.app"
```

The build targets your Mac's own architecture (Apple Silicon or Intel). The app is not Developer-ID signed or notarized; a locally built app opens normally from Finder. To install it, drag `P6 Vault.app` to `/Applications`. The app runs without a dev server, Java or an internet connection.

Development:

```bash
pnpm install
pnpm tauri dev           # app with hot reload
./scripts/check.sh       # fmt, clippy -D warnings, Rust tests, typecheck, Vitest, production web build
```

## Where your data lives

The app stores everything in the platform app-data folder (`~/Library/Application Support/app.p6vault.desktop/` on macOS):

| Path | Contents |
| --- | --- |
| `vault.sqlite` | Library, workspaces, undo history, snapshots, write journal (SQLite, WAL, `synchronous=FULL`) |
| `archives/` | Byte-identical copies of every imported `.syx` file |
| `backups/` | A verified 500-program `.syx` + JSON manifest for every write session. They are never deleted automatically. |
| `simulator/` | The separate Simulator-mode library and the simulated synth's memory |

Nothing is uploaded anywhere. Personal patches, databases, logs and backups are never committed; `fixtures/private/` is gitignored for your own test archives.

## Tests

| Command | What it covers |
| --- | --- |
| `cargo test --workspace` | 79 Rust tests: codec golden vectors and property tests, streaming parser at every split boundary, decoder offsets (name 107–126, arp 91, seq 93), fingerprints, import/export (589,000-byte bank), all bank-operation semantics including the spec's golden examples, 120-step undo across reopen, reconciliation, and 14 deployment integration tests against the simulator (drift, partial backup, mismatch, retries, stop, disconnect → inspect → restore/continue, crash after send-intent, hardware gate, no command 02 outside the WriteEngine) |
| `pnpm test` | Renderer logic: selection (range, toggle, select-all, filtering), search/sort/duplicate scoping, typing-target shortcut exclusion |
| `scripts/run-e2e.sh` | **Linux only.** Drives the real desktop binary through WebKitGTK WebDriver (`tauri-driver`) under Xvfb in Simulator mode, in 19 steps: connect, sync, import with preview, multi-select, drag to bank, undo/redo, Move to…, bulk category, protected audition + A/B, review → write → verify, export, restart persistence, drift reconciliation, and interrupted-write recovery. It needs `webkit2gtk-driver`, `xvfb` and `cargo install tauri-driver`. |

`cargo run -p p6-core --example make_fixtures -- fixtures/public` regenerates the synthetic public fixtures. They are clearly labelled `TEST …`, and neither the simulator nor the tests send them to real hardware.

## Documentation

- [docs/USER-GUIDE.md](docs/USER-GUIDE.md): connect, back up/sync, import, audition, organize, A/B, review/write, recover, export
- [docs/PROTOCOL.md](docs/PROTOCOL.md): framing, packing, offset namespace, timing and known limitations
- [docs/HARDWARE-TESTS.md](docs/HARDWARE-TESTS.md): safe step-by-step validation with your Prophet-6, with a results table
- [docs/STATUS.md](docs/STATUS.md): every required feature and gate, with its implementation and test status

## Layout

```
crates/p6-core/        pure Rust core (no Tauri/CoreMIDI): protocol, library, workspace ops,
                       storage (SQLite), device transactions + actor, simulator, deployment
src-tauri/             desktop shell: midir/CoreMIDI transport, typed IPC commands, events
src/                   React UI (Zustand store, TanStack Virtual tables)
scripts/               build-macos.sh, check.sh, run-e2e.sh + e2e.mjs
fixtures/public/       synthetic test data; fixtures/private/ is gitignored
docs/                  protocol, hardware tests, status, user guide
```
