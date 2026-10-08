use super::*;
use crate::workspace::SavedQuery;
impl DbxApp {
    pub(super) fn toggle_mcp(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if session.mcp_pairing.take().is_some() {
            self.show_toast(ToastKind::Info, "MCP sharing stopped; pairing revoked", cx);
            return;
        }
        if session.busy {
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        session.busy = true;
        let task = self.runtime.spawn(dbx_core::mcp::Pairing::start(engine));
        self.session_mut(id).unwrap().track_background_task(&task);
        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(id) else { return; };
                session.busy = false;
                match result {
                    Ok(pairing) => { session.mcp_pairing = Some(pairing); this.show_toast(ToastKind::Info, "Read-only MCP sharing enabled for this database. Copy pairing from the database menu; stop sharing there at any time.", cx); }
                    Err(error) => this.set_error(error.to_string()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
        cx.notify();
    }
    pub(super) fn copy_mcp_pairing(&mut self, id: SessionId, cx: &mut Context<Self>) {
        if let Some(pairing) = self.session(id).and_then(|s| s.mcp_pairing.as_ref()) {
            let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dbx"));
            cx.write_to_clipboard(ClipboardItem::new_string(
                serde_json::to_string_pretty(&pairing.recipe(&executable)).unwrap_or_default(),
            ));
            self.show_toast(ToastKind::Info, "Copied private MCP configuration. This grants read access while DBX is sharing; keep the token private.", cx);
        }
    }
    pub(super) fn show_mcp_activity(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pairing) = self.session(id).and_then(|s| s.mcp_pairing.as_ref()) else {
            return;
        };
        let log = pairing.activity();
        self.open_saved_query_for(id, SavedQuery { name: "MCP activity".into(),
            sql: format!("-- Read-only MCP sharing is active. Stop sharing in the database menu.\n-- Limits: one read, 100 rows, 1 MiB, 10 seconds. No SQL or write tools.\n{}", log.iter().map(|line| format!("-- {line}")).collect::<Vec<_>>().join("\n")) }, window, cx);
    }
}
