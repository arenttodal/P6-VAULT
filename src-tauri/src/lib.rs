//! P6 Vault desktop shell. All protocol, bank and write-permission logic lives in p6-core.

mod commands;
mod errors;
mod midi;
mod state;

use tauri::{Emitter, Manager};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            match state::AppState::new(dir) {
                Ok(s) => {
                    app.manage(s);
                    Ok(())
                }
                Err(e) => {
                    use tauri_plugin_dialog::DialogExt;
                    app.dialog().message(e.clone()).title("P6 Vault cannot start").blocking_show();
                    Err(e.into())
                }
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let st = window.state::<state::AppState>();
                let busy = st.busy.lock().unwrap().clone();
                if let Some((op, kind)) = busy {
                    if kind == "write" || kind == "prepare" || kind == "sync" || kind == "inspect" {
                        api.prevent_close();
                        let _ = window.emit("close-blocked", serde_json::json!({"op_id": op, "kind": kind}));
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::set_simulator_mode,
            commands::list_ports,
            commands::connection_status,
            commands::connect,
            commands::connect_simulator,
            commands::disconnect,
            commands::check_ports,
            commands::diagnostics,
            commands::preview_imports,
            commands::commit_imports,
            commands::discard_imports,
            commands::list_sources,
            commands::list_occurrences,
            commands::classification_detail,
            commands::list_workspaces,
            commands::workspace_view,
            commands::set_active_workspace,
            commands::create_workspace_from_source,
            commands::create_empty_workspace,
            commands::list_snapshots,
            commands::preview_op,
            commands::apply_op,
            commands::apply_meta,
            commands::preview_labels,
            commands::undo,
            commands::redo,
            commands::copy_programs,
            commands::clipboard_count,
            commands::export_bank,
            commands::export_selection,
            commands::export_source,
            commands::export_edit_buffer,
            commands::stop_operation,
            commands::sync_start,
            commands::apply_rebase,
            commands::prepare_review,
            commands::cancel_review,
            commands::write_confirmed,
            commands::list_write_sessions,
            commands::write_session_steps,
            commands::inspect_session,
            commands::recovery_rebase,
            commands::recovery_restore,
            commands::stage_backup_restore,
            commands::audition,
            commands::protect_edit_buffer,
            commands::protected_buffers,
            commands::restore_protected_buffer,
            commands::test_note,
            commands::panic,
            commands::get_setting,
            commands::set_setting,
            commands::backups_dir,
            commands::hardware_gate,
            commands::simulator_fault,
        ])
        .build(tauri::generate_context!())
        .expect("error while building P6 Vault")
        .run(|app, event| {
            // Cmd+Q / app quit: never exit silently in the middle of a hardware operation.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    if let Some(st) = app.try_state::<state::AppState>() {
                        if let Some((op, kind)) = st.busy.lock().unwrap().clone() {
                            api.prevent_exit();
                            let _ = app.emit("close-blocked", serde_json::json!({"op_id": op, "kind": kind}));
                        }
                    }
                }
            }
        });
}
