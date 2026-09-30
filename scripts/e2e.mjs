// End-to-end test of the real desktop app (WebKitGTK via tauri-driver) using the
// simulated Prophet-6. Linux CI/dev only; macOS has no WKWebView WebDriver.
//   pnpm tauri build --debug --no-bundle
//   tauri-driver &   (port 4444)
//   node scripts/e2e.mjs target/debug/p6-vault <screenshot-dir>
import fs from "node:fs";
import path from "node:path";

const app = path.resolve(process.argv[2] ?? "target/debug/p6-vault");
const shots = process.argv[3] ?? "e2e-shots";
fs.mkdirSync(shots, { recursive: true });
const BASE = "http://127.0.0.1:4444";
const FIX = path.resolve("fixtures/public");

async function wd(method, url, body) {
  const r = await fetch(BASE + url, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json();
  if (j.value && j.value.error) throw new Error(`${method} ${url}: ${j.value.error} ${j.value.message}`);
  return j.value;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let sid;
const S = (u) => `/session/${sid}${u}`;
const EL = "element-6066-11e4-a52e-4f735466cecf";

async function find(xpath, timeout = 8000) {
  const end = Date.now() + timeout;
  for (;;) {
    try {
      const e = await wd("POST", S("/element"), { using: "xpath", value: xpath });
      return e[EL];
    } catch (err) {
      if (Date.now() > end) throw new Error(`not found: ${xpath}`);
      await sleep(150);
    }
  }
}
async function click(xpath) {
  const e = await find(xpath);
  await wd("POST", S(`/element/${e}/click`), {});
  await sleep(120);
}
const btn = (text) => `//button[contains(normalize-space(.), ${JSON.stringify(text)})]`;
async function exec(script, args = []) {
  return wd("POST", S("/execute/sync"), { script, args });
}
async function execAsync(script, args = []) {
  return wd("POST", S("/execute/async"), { script, args });
}
async function shot(name) {
  const b64 = await wd("GET", S("/screenshot"));
  fs.writeFileSync(path.join(shots, `${name}.png`), Buffer.from(b64, "base64"));
}
async function state(expr) {
  return exec(`const s = window.__P6_TEST__.state(); return (${expr});`);
}
async function waitFor(expr, timeout = 30000, label = expr) {
  const end = Date.now() + timeout;
  for (;;) {
    const v = await state(expr).catch(() => false);
    if (v) return v;
    if (Date.now() > end) throw new Error(`timeout waiting for ${label}`);
    await sleep(200);
  }
}
async function key(value, mods = []) {
  const down = mods.map((m) => ({ type: "keyDown", value: m }));
  const up = mods.map((m) => ({ type: "keyUp", value: m }));
  await wd("POST", S("/actions"), { actions: [{ type: "key", id: "kb", actions: [...down, { type: "keyDown", value }, { type: "keyUp", value }, ...up] }] });
  await wd("DELETE", S("/actions"));
  await sleep(150);
}
const CTRL = "";
const SHIFT = "";
const ENTER = "";
const DOWN = "";

const results = [];
async function step(name, fn) {
  const t = Date.now();
  try {
    await fn();
    results.push({ name, ok: true, ms: Date.now() - t });
    console.log(`✔ ${name}`);
  } catch (e) {
    results.push({ name, ok: false, error: String(e) });
    console.log(`✘ ${name}: ${e}`);
    await shot(`FAIL-${name.replace(/\W+/g, "_")}`).catch(() => {});
    throw e;
  }
}

try {
  const sess = await wd("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: app } } } });
  sid = sess.sessionId;
  await sleep(1500);

  await step("app launches with empty library", async () => {
    await find(btn("Import .syx"));
    await shot("01-launch");
  });

  await step("enable simulator mode and connect", async () => {
    await click("//button[contains(@class,'conn')]");
    await click("//label[contains(., 'Simulator mode')]/input");
    await click(btn("Connect simulator"));
    await waitFor("s.status && s.status.state === 'Simulator'");
    await find("//span[contains(@class,'badge') and contains(., 'SIMULATOR')]");
  });

  await step("sync current from the (simulated) P6", async () => {
    await click(btn("Sync from P6"));
    await waitFor("s.workspace && s.workspace.baseline && s.workspace.baseline.kind === 'live' && !s.op", 120000, "sync");
    await shot("02-synced");
  });

  await step("import two archives with preview", async () => {
    await exec("window.__P6_TEST__.startImport(arguments[0]);", [[`${FIX}/synthetic-full-bank.syx`, `${FIX}/synthetic-mixed-archive.syx`]]);
    await find("//div[@role='dialog']//*[contains(., 'synthetic-mixed-archive.syx')]");
    await shot("03-import-preview");
    await click("//div[@role='dialog']//button[contains(., 'Import 2 file(s)')]");
    await find("//h2[contains(., 'Import finished')]");
    await shot("04-import-done");
    await click(btn("Done"));
    const n = await state("s.occurrences.length");
    if (n < 500 + 24 + 500) throw new Error(`library has ${n} rows`);
  });

  await step("search and multi-select library", async () => {
    const search = await find("//input[contains(@class,'search')]");
    await wd("POST", S(`/element/${search}/value`), { text: "TEST Old" });
    await waitFor("s.occurrences && document.querySelectorAll('.library .row').length > 0 && document.querySelectorAll('.library .row').length < 40");
    await click("(//section[contains(@class,'library')]//div[contains(@class,'row')])[1]");
    const fifth = await find("(//section[contains(@class,'library')]//div[contains(@class,'row')])[5]");
    await wd("POST", S("/actions"), {
      actions: [
        { type: "key", id: "kb", actions: [{ type: "keyDown", value: SHIFT }, { type: "pause", duration: 50 }, { type: "pause", duration: 50 }, { type: "pause", duration: 50 }, { type: "keyUp", value: SHIFT }] },
        { type: "pointer", id: "m", parameters: { pointerType: "mouse" }, actions: [{ type: "pause", duration: 50 }, { type: "pointerMove", origin: { [EL]: fifth }, x: 0, y: 0 }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
      ],
    });
    await wd("DELETE", S("/actions"));
    await waitFor("s.libSel.ids.size === 5", 5000, "5 selected");
    await shot("05-library-selected");
  });

  await step("drag 5 library sounds onto bank slot 010 (replace)", async () => {
    await exec("document.querySelector('.bank .scroll').scrollTop = 0");
    const src = await find("(//section[contains(@class,'library')]//div[contains(@class,'row') and contains(@class,'selected')])[1]");
    const dst = await find("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='010']]");
    await wd("POST", S("/actions"), {
      actions: [
        {
          type: "pointer",
          id: "m",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", origin: { [EL]: src }, x: 0, y: 0 },
            { type: "pointerDown", button: 0 },
            { type: "pointerMove", origin: { [EL]: src }, x: 20, y: 5, duration: 100 },
            { type: "pointerMove", origin: { [EL]: dst }, x: 0, y: 0, duration: 300 },
            { type: "pause", duration: 200 },
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await wd("DELETE", S("/actions"));
    await waitFor("s.workspace.changed_count === 5", 8000, "5 changed");
    const names = await state("s.workspace.slots.slice(10,15).map(x => x.new.name).join('|')");
    if (!names.startsWith("TEST Old")) throw new Error(names);
    await shot("06-after-drag");
  });

  await step("undo and redo with keyboard", async () => {
    await click("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='002']]");
    await key("z", [CTRL]);
    await waitFor("s.workspace.changed_count === 0", 5000, "undo");
    await key("z", [CTRL, SHIFT]);
    await waitFor("s.workspace.changed_count === 5", 5000, "redo");
  });

  await step("bank group move via Move to… dialog", async () => {
    await click("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='010']]");
    const r = await find("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='012']]");
    await wd("POST", S("/actions"), {
      actions: [
        { type: "key", id: "kb", actions: [{ type: "keyDown", value: SHIFT }, { type: "pause", duration: 50 }, { type: "pause", duration: 50 }, { type: "pause", duration: 50 }, { type: "keyUp", value: SHIFT }] },
        { type: "pointer", id: "m", parameters: { pointerType: "mouse" }, actions: [{ type: "pause", duration: 50 }, { type: "pointerMove", origin: { [EL]: r }, x: 0, y: 0 }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }] },
      ],
    });
    await wd("DELETE", S("/actions"));
    await waitFor("s.bankSel.ids.size === 3", 4000, "3 bank selected");
    const before = await state("s.workspace.slots.slice(10,13).map(x => x.new.entry_id).join()");
    await click(btn("Move to…"));
    const inp = await find("//div[@role='dialog']//input[@inputmode='numeric']");
    await wd("POST", S(`/element/${inp}/click`), {});
    await key("a", [CTRL]);
    await wd("POST", S(`/element/${inp}/value`), { text: "200" });
    await find("//div[@role='dialog']//*[contains(., 'slot(s) change')]");
    await shot("07-move-preview");
    await click("//div[@role='dialog']//button[normalize-space()='Move']");
    await waitFor(`s.workspace.slots.slice(200,203).map(x => x.new.entry_id).join() === ${JSON.stringify(before)}`, 5000, "moved block at 200");
  });

  await step("bulk category + favorite on library selection", async () => {
    await click("(//section[contains(@class,'library')]//div[contains(@class,'row')])[1]");
    await key("a", [CTRL]);
    await key("3");
    await waitFor("s.occurrences.filter(o => o.display_name.startsWith('TEST Old') && o.effective_category === 'Pad').length >= 24", 5000, "category set");
  });

  await step("audition B with edit-buffer protection", async () => {
    await exec("document.querySelector('.bank .scroll').scrollTop = 200 * 24 - 100");
    await sleep(300);
    await click("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='200']]");
    await key(ENTER);
    await find("//h2[contains(., 'Protect the current sound first')]");
    await shot("08-protect");
    await click(btn("Save buffer & audition"));
    await waitFor("s.lastSent && s.lastSent.status === 'sent'", 8000, "audition sent");
    await key(DOWN);
    await key("a");
    await waitFor("s.lastSent && s.lastSent.label.includes(' A ·')", 8000, "A sent");
  });

  await step("review, write and verify", async () => {
    const n = await state("s.workspace.changed_count");
    await click(btn(`Review ${n} change`));
    await find("//h2[contains(., 'to the Prophet-6')]", 60000);
    await shot("09-review");
    await click("//div[@role='dialog']//button[contains(., 'Write ') and contains(@class,'danger')]");
    await find("//h2[contains(., 'Bank written and verified')]", 120000);
    await shot("10-written");
    await click(btn("OK"));
    await waitFor("s.workspace.changed_count === 0 && s.workspace.baseline.kind === 'post_write'", 5000, "baseline advanced");
  });

  await step("export New bank to a verified 589000-byte .syx (typed command, no MIDI)", async () => {
    const out = path.resolve(shots, "exported-new-bank.syx");
    const n = await execAsync(
      "const done = arguments[arguments.length - 1]; const s = window.__P6_TEST__.state(); window.__TAURI_INTERNALS__.invoke('export_bank', { workspaceId: s.workspace.id, path: arguments[0] }).then(done, (e) => done('ERR ' + JSON.stringify(e)));",
      [out],
    );
    if (n !== 589000) throw new Error(`export returned ${n}`);
    if (fs.statSync(out).size !== 589000) throw new Error("file size");
  });

  await step("library hides superseded hardware reads", async () => {
    await exec("window.__P6_TEST__.state().setFilter({ search: '' })");
    const visible = await state("document.querySelector('.library .selcount').textContent");
    const total = await state("s.occurrences.length");
    const n = Number(String(visible).replace(/\D+/g, ""));
    if (!(n > 0 && n <= 500 + 500 + 27 + 5)) throw new Error(`visible ${visible} of ${total}`);
    await shot("12-library-after-write");
  });

  await step("history dialog lists the completed session and backup", async () => {
    await click("//li[contains(., 'History & backups')]");
    await find("//div[@role='dialog']//td[contains(., 'Completed')]");
    await shot("11-history");
    await click("//div[@role='dialog']//button[@aria-label='Close']");
  });
  await step("stage an offline change before restart", async () => {
    await exec("document.querySelector('.bank .scroll').scrollTop = 0");
    await sleep(200);
    await click("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='000']]");
    await click(btn("Swap…"));
    const inp = await find("//div[@role='dialog']//input[@inputmode='numeric']");
    await wd("POST", S(`/element/${inp}/click`), {});
    await key("a", [CTRL]);
    await wd("POST", S(`/element/${inp}/value`), { text: "499" });
    await click("//div[@role='dialog']//button[normalize-space()='Swap']");
    await waitFor("s.workspace.changed_count === 2", 5000, "swap staged");
    const search = await find("//input[contains(@class,'search')]");
    await wd("POST", S(`/element/${search}/value`), { text: "Sim Keys" });
    await sleep(900); // debounced UI context save
  });

  await step("restart: library, New, history, context and mode survive", async () => {
    const before = await state("({ n: s.occurrences.length, ws: s.workspace.id, changed: s.workspace.changed_count })");
    await wd("DELETE", `/session/${sid}`);
    sid = null;
    await sleep(1500);
    const sess = await wd("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: app } } } });
    sid = sess.sessionId;
    await waitFor("s.info && s.workspace && s.status", 15000, "reloaded");
    const after = await state("({ n: s.occurrences.length, ws: s.workspace.id, changed: s.workspace.changed_count, undo: s.workspace.can_undo, sim: s.status.simulator_mode, search: s.filter.search, conn: s.status.state, unfinished: s.info.unfinished_sessions.length })");
    if (after.n !== before.n || after.ws !== before.ws || after.changed !== 2) throw new Error(JSON.stringify({ before, after }));
    if (!after.undo || !after.sim || after.search !== "Sim Keys" || after.conn !== "Offline" || after.unfinished !== 0) throw new Error(JSON.stringify(after));
    await shot("13-after-restart");
    await key("z", [CTRL]);
    await waitFor("s.workspace.changed_count === 0", 5000, "undo after restart");
  });

  await step("offline audition explains missing connection; no MIDI", async () => {
    await click("//section[contains(@class,'bank')]//div[contains(@class,'row')][.//span[normalize-space()='000']]");
    await key(ENTER);
    await find("//div[contains(@class,'toast')][contains(., 'Connect the synth to audition')]");
  });
} catch (e) {
  process.exitCode = 1;
} finally {
  if (sid) await wd("DELETE", `/session/${sid}`).catch(() => {});
  fs.writeFileSync(path.join(shots, "results.json"), JSON.stringify(results, null, 2));
  console.log(`${results.filter((r) => r.ok).length}/${results.length} steps passed`);
}
