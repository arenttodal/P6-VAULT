# Status: 30 September 2026

**Summary.** Milestones 1–5 are implemented and tested against the simulator. Milestone 6 is done except for work that needs your Mac and your Prophet-6. The app was built and driven end-to-end on Linux (WebKitGTK) in Simulator mode. The macOS build runs in GitHub Actions (`.github/workflows/macos.yml`, macos-14 runner): fmt, clippy, all Rust tests (including the CoreMIDI build) and the renderer tests pass on macOS, and the job produces a universal `.app` + `.dmg` artifact. The app has not yet been launched from Finder on a real Mac (HARDWARE-TESTS step 9). Real-hardware validation is **Not tested** in every row (see HARDWARE-TESTS.md). Nothing in this document claims hardware verification.

Legend: ✅ implemented + automated test · 🟡 implemented, only partly tested or needs hardware/macOS confirmation · ⛔ not implemented

## Automated evidence
- `cargo test --workspace`: 65 unit tests (including a 10k-row performance test and the latest-only audition actor test) + 14 deployment integration tests. Clippy clean with `-D warnings`.
- `pnpm test`: 10 renderer tests. `pnpm typecheck` passes, and so does the production build.
- `scripts/run-e2e.sh`: 21/21 steps pass against the real Tauri binary (Linux/WebKitGTK, Simulator mode).
- Performance: listing 10,000 library occurrences takes 122 ms (release build, Linux container). Not yet measured on your Mac.

## Features

| Area | Requirement | Status | Evidence / notes |
| --- | --- | --- | --- |
| Connection | Enumerate ports, auto candidates, manual pairing | 🟡 | `midi.rs`; CoreMIDI not exercised here |
| | Validate a real P6 reply (identity and/or program read) before Connected | ✅ | `Device::probe`; sim tests |
| | Reconnect → new epoch invalidates permits/pending auditions | ✅ | `AppState::disconnect/next_epoch`; `confirm_rejects_wrong_epoch…` |
| | Unplug detection | 🟡 | port-presence poll every 3 s; no separate "Unresponsive" state (errors are reported per operation) |
| Reading | One program; all 500 with progress, retry, cancel | ✅ | `sync.rs`; e2e sync; `partial_read_is_not_a_snapshot` |
| | Partial reads kept, never a baseline/backup | ✅ | storage + `incomplete_backup_blocks_writes` |
| Archives | Multi-file import, drop, preview, provenance, immutable originals | ✅ | `library/import.rs`, `commit_import`; e2e import |
| | Program dumps, concatenated dumps, edit-buffer dumps (no invented slot) | ✅ | `mixed_file`, UI "Edit buffer #n" |
| | Unsupported formats explained (.p6lib/.p6program/MIDI) | ✅ | `preview_imports` |
| Library | Search, categories, favorites, duplicate groups, stable sort, multi-select | ✅ | `library.test.ts`, e2e |
| Audition | Command 03 only; latest-only queue; debounce 120 ms Auto (off by default) | ✅ | `actor.rs`, `audition_uses_command_03_only`, e2e |
| | Protect edit buffer before first load; restore; save to library | ✅ | `protect_edit_buffer`, History dialog, e2e |
| | "Confirmed loaded" via edit-buffer read-back in Diagnostics | ⛔ | Status shows Requested vs Sent only |
| | Test note (60/80/600 ms), Panic, app-owned note tracking | 🟡 | implemented; audible behaviour needs hardware |
| Staging | 500 slots, replace, moves (slot/gap), copy/paste, copy-to, swap, sort, revert, reset | ✅ | `operations.rs` golden tests; storage tests; e2e drag and move |
| | Overflow rejected atomically with highest valid start | ✅ | `replace_overflow_atomic`, UI drag ghost |
| | Stable entry IDs; copies get new IDs | ✅ | `library_replace_and_clipboard` |
| Group edits | Bulk category / clear override / favorite / Vault labels (prefix, suffix, numbering, preview) | ✅ | `meta_ops_undo_and_hashes_unchanged`, e2e |
| Comparison | A/Current, B/New, Same program, changed-only view | ✅ | e2e A/B |
| Deployment | Fresh full backup (.syx + manifest, re-verified) | ✅ | `backup.rs`; happy-path test checks 589,000 bytes |
| | Drift detection → reconcile; nothing written | ✅ | `drift_blocks_plan_and_reconciles`, e2e |
| | Frozen plan + hash, permit bound to plan/revision/epoch, edits frozen during review | ✅ | `plan.rs`, tests |
| | Only changed slots; re-read before write; exact read-back; bounded retries; stop on doubt | ✅ | `writer.rs` + 6 fault tests |
| | Final 500-slot reconciliation; baseline advances only on exact agreement | ✅ | `happy_path…`, `mismatch_stops…` |
| | Hardware gate: single-slot write + restore before multi-slot real writes | ✅ | `hardware_gate_limits_first_real_writes_to_one_slot` |
| Recovery | Durable journal (SendIntent committed before send); startup scan sends nothing | ✅ | `crash_after_send_intent_is_uncertain` |
| | Inspect; Continue; Restore affected; Keep hardware; Restore entire backup | ✅ | `disconnect_then_inspect_and_restore`, `continue_after_interruption`, e2e |
| | Quit/Cmd+Q during a hardware operation blocked with Keep running / Stop | 🟡 | implemented (`CloseRequested`, `ExitRequested`); not automated |
| Export | Full bank (589,000 bytes), selection with explicit destinations, original source, edit buffer; atomic + re-verified | ✅ | `export.rs` tests, e2e export |
| Persistence | Autosave per operation; ≥100 undo across restart; UI context; single-instance lock | ✅ | `hundred_undos_persist_across_reopen`, e2e restart |
| Classification | Local heuristic, 8 categories, reasons, score, manual override wins | ✅ | `classification.rs` tests |
| | Weights tuned on real archives | 🟡 | defaults from spec; tune with your library |
| Legacy | "Prepare compatible version" (derived payloads) | ⛔ | needs hardware evidence; exact verification stops safely if the firmware converts |
| Diagnostics | MIDI log (no payload bytes), simulator fault controls | ✅ | Diagnostics dialog |
| | Local log export | ⛔ | view only |
| Delivery | macOS `.app` + dmg | 🟡 | CI artifact (universal, ad-hoc signed, not notarized) or `scripts/build-macos.sh`; Finder launch to be confirmed |
| | README, PROTOCOL, HARDWARE-TESTS, USER-GUIDE, STATUS | ✅ | `docs/` |

## Deviations from the spec (deliberate)
- **Drag and drop** is a small custom pointer-drag module (`components/dragging.ts`), not dnd-kit, because the tables are virtualized. It supports frozen ordered selection, autoscroll near the edges, Escape to cancel, an insertion line and a replacement highlight. Every drag has a dialog/keyboard alternative.
- **Renderer E2E** uses WebDriver against the real desktop binary (`tauri-driver`, Linux) instead of Playwright in a browser. This tests the real IPC and backend. macOS WKWebView has no WebDriver, so native macOS smoke tests stay manual (HARDWARE-TESTS step 9).
- **Clipboard** is internal to the app: a versioned Vault bundle validated in Rust. It never reads system-clipboard text as programs. Text fields keep native copy/paste.
- **Library view** hides superseded hardware reads (older syncs, pre-write backups) from "All sounds". They are still in the Vault under "Hardware reads & backups". Duplicate counts are computed within the visible scope.
- **Imported-baseline workspaces:** when a bank made from an imported archive is first reconciled with the synth, every slot where New differs from the synth needs an explicit Keep New / Use Synth choice. The app does not assume the archive reflects the synth.

## Milestone gates
| Milestone | Gate | Status |
| --- | --- | --- |
| 1 Core + import | Real .syx → named rows; core tests | ✅ (synthetic fixtures; test with your own archives) |
| 2 Offline manager | Build/reopen/export a complete bank without hardware | ✅ |
| 3 Read + audition | Simulator flow passes; physical matrix ready | ✅ / hardware pending |
| 4 Group workflow | Group ops + restart persistence | ✅ |
| 5 Deployment | Fault tests pass; physical one-slot gate ready | ✅ / hardware pending |
| 6 Product | Finder-launchable app; validation matrix | 🟡 build on your Mac; matrix in HARDWARE-TESTS.md |
