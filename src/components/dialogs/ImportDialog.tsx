import { useState } from "react";
import { api } from "../../api/backend";
import type { CommitResult, ImportPreview } from "../../api/types";
import { useApp } from "../../stores/app";
import { Modal } from "../Modal";

export function ImportDialog({ previews }: { previews: ImportPreview[] }) {
  const { closeDialog, openDialog, refreshLibrary, error } = useApp();
  const ok = previews.filter((p) => !p.error);
  const [chosen, setChosen] = useState<Set<string>>(new Set(ok.map((p) => p.token)));
  const [busy, setBusy] = useState(false);

  const cancel = () => {
    void api().discardImports(ok.map((p) => p.token));
    closeDialog();
  };

  const commit = async () => {
    setBusy(true);
    try {
      const results: CommitResult[] = await api().commitImports([...chosen]);
      void api().discardImports(ok.map((p) => p.token).filter((t) => !chosen.has(t)));
      await refreshLibrary();
      openDialog({ kind: "importResults", results, previews });
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={`Import ${previews.length} file(s)`}
      onClose={cancel}
      wide
      footer={
        <>
          <span className="muted">Importing never sends MIDI. Original files are kept unchanged in the Vault.</span>
          <button onClick={cancel}>Cancel</button>
          <button className="primary" disabled={busy || chosen.size === 0} onClick={() => void commit()}>
            Import {chosen.size} file(s)
          </button>
        </>
      }
    >
      <table className="simple">
        <thead>
          <tr>
            <th />
            <th>File</th>
            <th>Programs</th>
            <th>Edit buffers</th>
            <th>Unique</th>
            <th>Repeated in file</th>
            <th>Already in Vault</th>
            <th>Excluded</th>
          </tr>
        </thead>
        <tbody>
          {previews.map((p) => (
            <tr key={p.path}>
              <td>
                {!p.error && (
                  <input
                    type="checkbox"
                    aria-label={`import ${p.file_name}`}
                    checked={chosen.has(p.token)}
                    onChange={(e) => {
                      const n = new Set(chosen);
                      if (e.target.checked) n.add(p.token);
                      else n.delete(p.token);
                      setChosen(n);
                    }}
                  />
                )}
              </td>
              <td>
                <b>{p.file_name}</b>
                {p.error && <div className="error">{p.error.message}</div>}
                {p.complete_user_bank && <div className="muted small">Complete 000–499 bank</div>}
                {p.repeated_addresses > 0 && <div className="warn small">{p.repeated_addresses} address(es) appear more than once — kept as separate entries</div>}
                {p.noncanonical > 0 && <div className="warn small">{p.noncanonical} frame(s) use non-canonical packing; original bytes preserved</div>}
              </td>
              <td>{p.programs}</td>
              <td>{p.edit_buffers}</td>
              <td>{p.unique_payloads}</td>
              <td>{p.repeated_in_file}</td>
              <td>{p.already_in_vault}</td>
              <td>
                {p.excluded.length ? (
                  <details>
                    <summary>{p.excluded.length}</summary>
                    <ul className="small">
                      {p.excluded.slice(0, 20).map((x) => (
                        <li key={x.message_index}>
                          #{x.message_index}: {x.kind} – {x.reason}
                        </li>
                      ))}
                    </ul>
                  </details>
                ) : (
                  0
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Modal>
  );
}

export function ImportResultsDialog({ results, previews }: { results: CommitResult[]; previews: ImportPreview[] }) {
  const { closeDialog, refreshWorkspace, workspace, error, toast } = useApp();
  const names = new Map(previews.map((p) => [p.token, p.file_name]));
  const buildBank = async (sourceId: string, name: string) => {
    try {
      const ws = await api().createWorkspaceFromSource(sourceId, name);
      await api().setActiveWorkspace(ws);
      await refreshWorkspace();
      toast("success", "New bank created from the imported file.");
      closeDialog();
    } catch (e) {
      error(e);
    }
  };
  return (
    <Modal
      title="Import finished"
      onClose={closeDialog}
      footer={
        <button className="primary" onClick={closeDialog}>
          Done
        </button>
      }
    >
      <ul className="stack">
        {results.map((r) => (
          <li key={r.token}>
            <b>{names.get(r.token)}</b>:{" "}
            {r.summary ? (
              <>
                {r.summary.programs} programs · {r.summary.unique_payloads} unique · {r.summary.repeated_in_file} repeated in file · {r.summary.already_in_vault} already
                in Vault · {r.summary.excluded} excluded
                {r.summary.previously_imported_file && <span className="muted"> (this exact file was imported before)</span>}
                {r.summary.complete_user_bank && (
                  <div>
                    <button className="mini" onClick={() => void buildBank(r.summary!.source_id, r.summary!.file_name)}>
                      {workspace ? "Open as a separate bank" : "Use as my starting bank"}
                    </button>
                  </div>
                )}
              </>
            ) : (
              <span className="error">{r.error?.message}</span>
            )}
          </li>
        ))}
      </ul>
    </Modal>
  );
}
