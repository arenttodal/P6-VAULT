# Hardware validation with your Prophet-6

Do these steps with the synth connected, in order. Nothing here requires deleting or overwriting anything you care about. **You choose the test slot**; the app never picks one for you. Record results in the table at the end. Until steps 6a and 6b pass, the app only allows real writes that change exactly one slot (the badge "Hardware test 0/2" in the toolbar).

Before you start: back up the synth your usual way too (e.g. a SoundTower or manual dump), make sure no other librarian is running, and have your normal audio monitoring on.

## 1. Connection and single reads
1. Connect the P6 over USB. In the P6's Globals set **MIDI SysEx** to USB.
2. In P6 Vault, click the connection button. Turn Simulator mode **off**. The Prophet should be pre-selected on both input and output. Click **Connect & verify**.
   - Expected: "Connected · Usb". Open **MIDI diagnostics** and check that you see `identity inquiry`, `identity reply ok` and `request program 000` → `program 000 ok`.
3. If connecting fails, note the error, and try the manual port choices and the setup-help checklist.

## 2. Full baseline read and export
1. Click **Sync from P6**. Expected: 500/500. This takes roughly 20–40 s on USB (to be measured; note the time).
2. Check a few names in the Current column against the synth's display, e.g. slots 000, 099, 100, 499.
3. Click **Export New** and save the file. Check that it is 589,000 bytes. Optional: send it to the synth with another tool **only if you really mean to restore that bank**; you don't need to.

## 3. Edit-buffer protection and audition
1. On the synth, change a knob on the current sound so the edit buffer holds an unsaved edit.
2. In the Bank, click a slot and press **Enter**. Expected: the "Protect the current sound first" dialog. Click **Save buffer & audition**. The synth should now play the selected sound.
3. Audition a few imported patches, including one with an arpeggiator or sequence (shown with an `Arp`/`Seq` badge).
4. Open **History & backups → Protected edit buffers → Restore to synth**. Expected: your unsaved tweak is back.
5. Listen through your normal monitoring and note anything odd (level jumps, stuck notes). Try **Test note** and **Panic**.

## 4. A/B and "audition never stores"
1. Stage a change in one slot (drag a library sound onto it). Press **A** and **B**, and use the buttons, to switch between Current and New.
2. Click **Sync from P6** again. Expected: "Current refreshed…", and no slot differs from before. This proves the auditions changed nothing in stored memory.

## 5. Decoder spot-checks
Pick three programs you know well:
- The names match, including trailing spaces and odd characters.
- A program with the arpeggiator on shows `Arp`; one with the sequencer on shows `Seq`.
- Change the amp attack on the synth, store it to your **chosen test slot**, sync, and see whether the category suggestion shifts (e.g. towards Pad).
- Hover a library name to see its format byte (payload offset 105). Note the values you see for current and old archives. Unsupported layouts would show fallback names like `file.syx #012`.

## 6. Single-slot write and restore (the hardware gate)
**6a. Write:** choose one user slot whose sound you can afford to change temporarily, e.g. a spare slot like 499. Stage exactly one change there (drag a library sound onto it). Click **Review 1 change** and check the review: it lists exactly that slot and the backup path. Click **Write 1 program**.
- Expected: "Bank written and verified". The synth now has the new sound at that slot; check it on the synth.

**6b. Restore:** open **History & backups** and click **Restore backup…** on the session from 6a. This stages the pre-write backup as a separate bank ("Restore of backup"), and its only difference from the synth is your test slot. Review (1 change) → Write.
- Expected: verified; the original sound is back on the synth. The badge disappears (2/2).

## 7. Small cross-bank group, stop and disconnect
1. Stage about 6 changes across the 099/100 boundary (e.g. swap 097–099 with 100–102). Review → Write. Expected: verified.
2. Stage another small group and start the write. After the first program, click **Stop after current program**. Expected: "Write stopped", with Verified/Not attempted counts. Open **Inspect & recover…** → Inspect. Choose **Restore affected slots…** or **Continue deployment…**, review, and write.
3. Optional and only if comfortable: during a write of a small group, unplug USB. Expected: the write stops. Reconnect → History & backups → Recover… → Inspect → choose an action. No slot outside the plan changes.

## 8. Timing
Note the sync and write durations from steps 2 and 7. If you have a DIN MIDI interface, repeat step 1 with the **DIN** box ticked and a sync. Otherwise record DIN as *Not tested*.

## 9. Packaged app and restart
1. Quit, then launch `P6 Vault.app` from Finder (not from a terminal). Expected: the library, New bank, undo history (Undo still enabled), backups and your search text are all back.
2. Kill the app during a write (Activity Monitor → Force Quit) only if you want to test crash recovery. On relaunch it should show "An earlier write did not finish" and send nothing until you inspect.

## Results

| # | Check | Result (Passed / Failed / Not tested) | Notes |
| --- | --- | --- | --- |
| 1 | Discovery, identity/program read | Not tested | |
| 2 | Full 500 read, export 589,000 bytes | Not tested | |
| 3 | Edit-buffer protection, audition, restore | Not tested | |
| 4 | A/B; audition does not change stored memory | Not tested | |
| 5 | Names, Arp/Seq flags, envelope, version byte | Not tested | |
| 6a | Single-slot verified write | Not tested | |
| 6b | Single-slot verified restore | Not tested | |
| 7 | Cross-bank group, stop, disconnect recovery | Not tested | |
| 8 | USB timing measured; DIN | Not tested | |
| 9 | Finder launch, restart persistence | Not tested | |
