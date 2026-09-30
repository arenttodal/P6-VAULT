import { useEffect, useState } from "react";
import { api } from "../../api/backend";
import type { PortList } from "../../api/types";
import { useApp } from "../../stores/app";
import { Modal } from "../Modal";

export function ConnectDialog() {
  const { status, closeDialog, set, error, toast, refreshWorkspace, refreshLibrary } = useApp();
  const [ports, setPorts] = useState<PortList | null>(null);
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [din, setDin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [portErr, setPortErr] = useState<string | null>(null);

  const load = async () => {
    try {
      const p = await api().listPorts();
      setPorts(p);
      setInput((cur) => cur || p.suggested_input || p.inputs[0] || "");
      setOutput((cur) => cur || p.suggested_output || p.outputs[0] || "");
      setPortErr(null);
    } catch (e) {
      setPortErr((e as { message?: string }).message ?? String(e));
    }
  };
  useEffect(() => {
    void load();
  }, []);

  const connected = status?.state === "Connected" || status?.state === "Simulator";
  const sim = !!status?.simulator_mode;

  const run = async (f: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    try {
      await f();
      set({ status: await api().connectionStatus() });
      toast("success", ok);
      closeDialog();
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  const switchMode = async (v: boolean) => {
    setBusy(true);
    try {
      const st = await api().setSimulatorMode(v);
      set({ status: st, libSel: { ids: new Set(), anchor: null, focus: null }, bankSel: { ids: new Set(), anchor: null, focus: null }, requested: null, lastSent: null });
      await Promise.all([refreshWorkspace(), refreshLibrary()]);
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title="Connection" onClose={closeDialog}>
      <p className="muted">
        Status: <b>{status?.state}</b> {status?.description && `· ${status.description}`}
      </p>
      <label className="toggle">
        <input type="checkbox" checked={sim} disabled={busy} onChange={(e) => void switchMode(e.target.checked)} /> Simulator mode (separate library; try the app
        without hardware)
      </label>
      {sim ? (
        <div className="stack">
          <p className="muted">
            The simulated Prophet-6 holds 500 synthetic test programs and remembers what you write to it. Nothing is sent to real hardware, and nothing done here
            counts as hardware validation.
          </p>
          <div className="row-buttons">
            <button className="primary" disabled={busy} onClick={() => void run(() => api().connectSimulator(), "Simulator connected.")}>
              Connect simulator
            </button>
            {connected && (
              <button disabled={busy} onClick={() => void run(() => api().disconnect(), "Disconnected.")}>
                Disconnect
              </button>
            )}
          </div>
        </div>
      ) : (
        <div className="stack">
          {portErr && <p className="error">{portErr}</p>}
          <div className="grid2">
            <label>
              MIDI input (from the P6)
              <select value={input} onChange={(e) => setInput(e.target.value)}>
                {ports?.inputs.length === 0 && <option value="">No MIDI inputs found</option>}
                {ports?.inputs.map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </label>
            <label>
              MIDI output (to the P6)
              <select value={output} onChange={(e) => setOutput(e.target.value)}>
                {ports?.outputs.length === 0 && <option value="">No MIDI outputs found</option>}
                {ports?.outputs.map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </label>
          </div>
          <label className="toggle">
            <input type="checkbox" checked={din} onChange={(e) => setDin(e.target.checked)} /> 5-pin DIN MIDI interface (slower timing; needs both MIDI cables)
          </label>
          <div className="row-buttons">
            <button onClick={() => void load()} disabled={busy}>
              Rescan ports
            </button>
            <button className="primary" disabled={busy || !input || !output} onClick={() => void run(() => api().connect(input, output, din), "The Prophet-6 answered. Connected.")}>
              {busy ? "Checking…" : "Connect & verify"}
            </button>
            {connected && (
              <button disabled={busy} onClick={() => void run(() => api().disconnect(), "Disconnected.")}>
                Disconnect
              </button>
            )}
          </div>
          <details>
            <summary>Setup help</summary>
            <ul>
              <li>On the Prophet-6: Globals → MIDI SysEx → choose the port you use (USB or MIDI).</li>
              <li>USB: the synth appears as “Prophet 6”. DIN: connect both MIDI Out→In and In←Out.</li>
              <li>Quit other librarians and avoid MIDI loops or merges.</li>
              <li>Sound comes from the Prophet's audio outputs; this app only loads patches.</li>
              <li>“Connected” is shown only after the synth answers an identity request or a program read.</li>
            </ul>
          </details>
        </div>
      )}
    </Modal>
  );
}
