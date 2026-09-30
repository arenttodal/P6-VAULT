# Prophet-6 protocol notes (as implemented)

Sources: [S1] Prophet-6 Operation Manual v2.1, Appendix C (MIDI implementation). [S2] Sequential's "P6 packed parameter data assignments" chart (Product Designer forum reply, 24 May 2016). The code for everything below is in `crates/p6-core/src/protocol/`.

## Messages

| Operation | Frame | Bytes | Where |
| --- | --- | --- | --- |
| Request stored program | `F0 01 2D 05 bank prog F7` | 7 | `messages::request_program` |
| Stored-program data | `F0 01 2D 02 bank prog <1171 packed> F7` | 1178 | parse: `parse_message`; file export: `program_file_frame`; hardware transmission: `stored_write_frame` (crate-private, WriteEngine only) |
| Request edit buffer | `F0 01 2D 06 F7` | 5 | `request_edit_buffer` |
| Edit-buffer data / audition | `F0 01 2D 03 <1171 packed> F7` | 1176 | `edit_buffer_frame` |
| Identity inquiry | `F0 7E 7F 06 01 F7` | 6 | `identity_inquiry` |

- Parsing accepts banks 0–9 and programs 0–99 (addresses 000–999). The `UserSlot` type (0–499) is the only type the write path accepts. Addresses are shown as three digits: `bank = slot / 100`, `program = slot % 100`.
- Identity replies are parsed with a variable length. Manufacturer `01` and family `2D` are checked, and the remaining bytes are kept as the version, as received. No firmware version string is invented from them.
- `Device::send_frame` refuses any outgoing command-02 frame unless it comes from the WriteEngine path, which requires a `ConfirmedWritePermit`. There is no IPC command that sends raw bytes.

## Packing (7-in-8)

Each group of up to 7 raw bytes becomes one prefix byte (bit *i* = bit 7 of raw byte *i*) followed by the raw bytes masked with `0x7F`. 1024 raw bytes = 146 full groups + one 2-byte group = 1171 packed bytes. The canonical encoder zeroes unused prefix bits. When an import has a noncanonical final prefix, the payload is still decoded, the original frame is kept, and the row is flagged `NC`. The payload bytes are the same either way, and the app never re-sends the original frame. Golden vectors (`80 01 FF 7F 00 81 02` → `25 00 01 7F 7F 00 01 02`; `FF 80` → `03 7F 00`) and property tests are in `packing.rs`.

## Streaming parser

`framing::FrameAssembler` is shared by file import (1 MiB frame bound) and MIDI input (4 KiB bound). It handles:

- concatenated frames and fragmentation at any byte boundary
- interleaved real-time bytes (`F8`–`FF`), which are never stored as payload
- a nested `F0`, which abandons the prior frame
- a non-real-time status byte inside a frame, which abandons it
- truncation at end of input, reset or timeout
- oversize frames, which are bounded and reported

Every message field is validated before any slicing: manufacturer, model, command, address, terminator and exact length.

## Payload layout: NRPN numbers are not offsets

All offsets are zero-based in the **unpacked** 1024-byte payload, as given by the [S2] chart. They are not NRPN numbers, and not positions in the packed stream or the MIDI frame.

| Field | Offset |
| --- | --- |
| Name (20 ASCII bytes) | 107–126 |
| Arpeggiator on | 91 |
| Sequencer on | 93 |
| Format version / editor byte | 105 / 106 |
| Sequence note/velocity data | 128–895 |

The full field map is in `payload.rs` (the `fields!` table). All 1024 bytes, including reserved bytes, are preserved and hashed.

- `exact_hash = SHA-256(payload)`
- `name_independent_hash = SHA-256(payload with 107..127 zeroed)`. This means "identical except for the name"; it is not an acoustic similarity measure.

**Layout validation:** the chart layout is assumed for every payload whose name bytes are all 7-bit. If any name byte has bit 7 set, the payload is treated as an unsupported layout: its bytes are kept, it gets a fallback label (`<source> #<address>`), its parameter classification is unavailable, and it has no name-independent hash. Format-version byte values have **not** yet been checked against real dumps (see HARDWARE-TESTS step 5).

**Normalization ranges** used only by the category heuristic come from NRPN documentation ranges (e.g. envelopes 0–127, cutoff 0–164). They are evidence, not a validated raw encoding. Raw bytes are never clamped or rewritten.

## Timing (engineering defaults, `device/transport.rs`)

| Profile | Read timeout | Retries | Inter-request | Post-write settle | Wire rate |
| --- | --- | --- | --- | --- | --- |
| USB | 2.5 s | 2 | 30 ms | 120 ms | not rate-limited |
| DIN | 4 s | 2 | 50 ms | 120 ms | 31,250 bit/s (a 1178-byte frame ≈ 377 ms; reading 500 programs ≥ 188 s) |

These are starting values to tune on your synth; they are not manufacturer guarantees. On DIN, the app waits for the computed wire time after each send.

## Transactions and limitations

- A single MIDI actor thread owns the connection and serializes every transaction. Auditions are latest-only: a pending audition is replaced, never queued. They are blocked and purged during reads, preparation, writes and recovery.
- The MIDI input callback only copies bytes into a bounded queue (8192 messages). If that queue overflows during a transaction, the transaction fails; data is never silently dropped.
- P6 SysEx has no transaction ID. Replies are matched by command and address only. After a timeout, and before every write verification, the input is drained, so a late reply for the same address sent before a write cannot be taken as the verification. If a reply is ambiguous, the write stops (fail closed).
- The P6 has no lock or compare-and-swap. A 500-slot read is a sequential observation. The WriteEngine re-reads each slot immediately before writing it, but a change on the synth in the tiny gap between that read and the write cannot be detected.
- A successful send only means the bytes were handed to the OS. A write counts as verified only after the slot is read back and all 1024 bytes match exactly.

## Legacy/firmware conversion

This part is not implemented yet and needs hardware evidence. Imported payloads, including their version bytes, are kept exactly as they are and never rewritten. If a real synth turns out to convert old payloads, writes of those payloads will fail verification (exact comparison) and stop safely. The spec's "Prepare compatible version" flow (load, read back, accept a derived version) would then be the next thing to implement.
