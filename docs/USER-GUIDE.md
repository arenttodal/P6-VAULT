# Using P6 Vault

## The screen
- **Top bar:** connection status (click it to connect), Undo/Redo, Import .syx, Sync from P6, Export New, and **Review N changes**.
- **Left sidebar:** library filters (favorites, duplicates, unclassified), categories, sources (your imported files), older hardware reads and backups, your banks, and History & backups.
- **Library (middle):** every sound you have imported or read, one row per occurrence. The same sound from two files appears twice, marked `=1` in the Dup column.
- **Bank (right):** the 500 user slots in physical order. **A · Current** is what was last read from the synth; **B · New** is what you are building. Changed slots are marked ● with an amber slot number.
- **Bottom bar:** the focused slot or sound, A / B / Audition, Auto audition, Test note, Panic, and the progress of any running operation with a Stop button.

## 1. Connect
Click the status button. Turn off Simulator mode, choose the Prophet (it is pre-selected when found), and click **Connect & verify**. The app only reports Connected after the synth answers. For a 5-pin MIDI interface, tick **DIN** and connect both cables.

## 2. Back up / sync
Click **Sync from P6** to read slots 000–499. The first complete read becomes **Current**, and **New** starts as a copy of it. A later sync refreshes Current while keeping your staged changes. If a slot changed on the synth *and* you changed it in New, you choose Keep New or Use Synth for that slot. If a sync is incomplete, it is kept as a separate source, and you can **Retry missing** slots.

## 3. Import old archives
Click **Import .syx** or drop files anywhere on the window. The preview shows programs, edit buffers, unique and repeated sounds, sounds already in the Vault, and excluded messages with reasons. Importing never sends MIDI, and the original files are kept byte-for-byte. `.p6lib`/`.p6program`/MIDI files are not supported; export them to `.syx` first. A complete 000–499 bank file can become a separate bank (**Bank** button next to the source), so you can organize offline before connecting.

## 4. Audition
Click a sound or slot and press **Enter** (or double-click). The first time, the app saves the synth's current edit buffer, including unsaved tweaks, so nothing is lost. Auditioning only loads the edit buffer; stored programs never change. You can restore the saved buffer from History & backups. **Auto** auditions as you move with the arrow keys. **Test note** plays middle C for modules; **Panic** releases notes.

## 5. Organize
- **Select:** click; ⌘-click toggles; ⇧-click selects a range; ⌘A selects all results in the focused pane (including rows scrolled out of view); ←/→ switch panes; ↑/↓ move focus.
- **Library → New:** drag the selection onto a bank slot, or use **Place at NNN** or ⌘C/⌘V. The sounds replace the slots from there on and nothing shifts. If the group would run past 499, the drop is refused and the latest valid start slot is shown.
- **Rearrange New:** drag selected slots between rows; an insertion line shows where they will go. Or use **Move to…**: the block starts at the chosen slot and everything else closes up in order, so nothing is lost or duplicated. **Copy to…** duplicates slots elsewhere. **Swap…** exchanges two equal ranges. **Sort A–Z** and **Group by category** sort only the selected slots, in place.
- **Revert to Current** resets selected slots; **Reset…** resets the whole bank.
- **Categories, favorites and labels:** set a category from the menu or with keys **1–8**; ★ toggles favorite; **Labels…** adds a prefix/suffix/numbering. These are Vault notes only; they don't rename programs on the synth or count as changes to write.
- **Undo/Redo** (⌘Z / ⇧⌘Z) cover every change, even after a restart.

## 6. Compare A/B
Focus a bank slot and press **A** (Current) or **B** (New), or use the buttons. If both are identical, the app says "Same program". Tick **Changed only** to see just the slots that will be written.

## 7. Review and write
Click **Review N changes**. The app first reads all 500 slots and saves a verified backup (`.syx` + manifest). If the synth changed since your last sync, you reconcile first and nothing is written. The review lists every slot that will change, with old and new names, per-bank counts, the backup location and an estimated time. Click **Write N programs** to go ahead. Each slot is re-read just before writing, written, and read back to check an exact match; at the end all 500 slots are read again. Please don't use the synth's front-panel Write or another librarian while this runs. **Stop after current program** stops safely.

Until you finish the single-slot hardware test ([HARDWARE-TESTS.md](HARDWARE-TESTS.md), step 6), real writes are limited to one changed slot.

## 8. Recover
If a write is interrupted (stop, unplug, crash), the app shows **An earlier write did not finish** and sends nothing on its own. Connect, click **Inspect** (reads the synth), then choose:
- **Continue deployment:** write only what is still missing.
- **Restore affected slots:** stage the pre-write sounds back in a separate bank.
- **Keep the synth as it is:** accept the current state and keep organizing.

Each option leads to the normal review with a fresh backup and your confirmation. **History & backups → Restore backup…** stages any complete backup the same way.

## 9. Export
**Export New** saves all 500 programs (589,000 bytes, addresses 000–499). Library **Export…** saves selected sounds with destination slots you choose. The ⇩ button next to a source exports the original file unchanged. Protected edit buffers can be exported from History. Exporting never sends MIDI. Note that sending an addressed file to the synth with another tool writes those slots.
