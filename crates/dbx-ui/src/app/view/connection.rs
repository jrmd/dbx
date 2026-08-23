use super::super::*;

impl DbxApp {
    pub(super) fn render_connection(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let url_focus = self.draft.connection_editor.read(cx).focus_handle();
        let host_focus = self.draft.host_editor.read(cx).focus_handle();
        let port_focus = self.draft.port_editor.read(cx).focus_handle();
        let username_focus = self.draft.username_editor.read(cx).focus_handle();
        let password_focus = self.draft.password_editor.read(cx).focus_handle();
        let database_focus = self.draft.database_editor.read(cx).focus_handle();
        let kind = self.draft.kind;
        let details =
            self.draft.mode == ConnectionFormMode::Details && kind != DatabaseKind::SQLite;
        let environment = self.draft.environment;
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;

        div().flex_1().min_h_0().flex().bg(THEME.canvas)
            .child(
                div().w(if self.compact_layout { px(0.) } else { px(252.) }).flex_none()
                    .when(self.compact_layout, |view| view.overflow_hidden())
                    .flex().flex_col().border_r_1().border_color(THEME.border).bg(THEME.panel)
                    .child(div().h(px(54.)).px(px(14.)).flex().items_center().justify_between().border_b_1().border_color(THEME.border)
                        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Connections"))
                        .child(div().id("new-connection-from-list").size(px(26.)).rounded(px(6.)).flex().items_center().justify_center().bg(THEME.accent).cursor_pointer().child(icon(Icon::Add, THEME.text)).on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx)))))
                    .child(div().id("saved-connections").flex_1().min_h_0().overflow_y_scroll().p(px(8.)).flex().flex_col().gap(px(3.))
                        .when(saved_connections.is_empty(), |view| view.child(div().p(px(10.)).text_size(px(11.)).text_color(THEME.text_muted).child("No saved connections yet.")))
                        .children(saved_connections.into_iter().map(|profile| {
                            let id = profile.id; let selected = selected_profile == Some(id); let choose = profile.clone();
                            div().id(SharedString::from(format!("saved-connection-{id}"))).h(px(48.)).px(px(9.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel }).cursor_pointer().flex().items_center().gap(px(8.))
                                .child(database_logo(profile.kind, if selected { THEME.accent } else { THEME.text_muted }))
                                .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().text_size(px(11.)).child(profile.name)).child(div().truncate().text_size(px(9.)).text_color(THEME.text_muted).child(display_url(&profile.url))))
                                .child(environment_badge(profile.environment))
                                .on_click(cx.listener(move |this, _, _, cx| this.select_saved_connection(choose.clone(), cx)))
                        }))),
            )
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().id("connection-form-scroll").flex_1().min_h_0().overflow_y_scroll().p(if self.compact_layout { px(14.) } else { px(24.) }).flex().justify_center()
                    .child(div().w_full().max_w(px(720.)).flex().flex_col().gap(px(14.))
                        .child(div().flex().items_end().justify_between().child(div().flex().flex_col().gap(px(3.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if self.draft.selected_profile.is_some() { "Edit connection" } else { "New connection" })).child(div().text_size(px(11.)).text_color(THEME.text_muted).child("Configure a saved profile or connect once."))).child(div().flex().items_center().gap(px(6.)).px(px(8.)).py(px(4.)).rounded_full().bg(THEME.panel_raised).child(database_logo(kind, THEME.accent)).child(div().text_size(px(10.)).font_weight(FontWeight::MEDIUM).text_color(THEME.accent).child(kind.to_string()))))
                        .child(div().rounded(px(9.)).border_1().border_color(THEME.border).bg(THEME.panel).p(px(16.)).flex().flex_col().gap(px(12.))
                            .child(div().flex().gap(px(6.)).children([DatabaseKind::PostgreSQL, DatabaseKind::MySQL, DatabaseKind::SQLite, DatabaseKind::Redis].into_iter().map(|option| { let selected = option == kind; div().id(SharedString::from(format!("engine-{option}"))).flex().items_center().gap(px(5.)).px(px(9.)).py(px(7.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel_raised }).text_color(if selected { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child(database_logo(option, if selected { THEME.accent } else { THEME.text_muted })).child(option.to_string()).on_click(cx.listener(move |this, _, _, cx| this.select_kind(option, cx))) })))
                            .child(div().flex().items_center().gap(px(6.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Environment")).children(ConnectionEnvironment::ALL.into_iter().map(|option| { let selected = option == environment; div().id(SharedString::from(format!("environment-{option}"))).flex().items_center().gap(px(5.)).px(px(9.)).py(px(7.)).rounded(px(6.)).bg(if selected { THEME.accent_soft } else { THEME.panel_raised }).text_color(if selected { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child(div().size(px(6.)).rounded_full().bg(environment_color(option))).child(option.to_string()).on_click(cx.listener(move |this, _, _, cx| this.select_environment(option, cx))) })))
                            .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Connection name")).child(editor::input(self.draft.connection_name_editor.clone(), name_focus, false)))
                            .when(kind != DatabaseKind::SQLite, |view| view.child(div().flex().gap(px(5.))
                                .child(div().id("connection-details-mode").flex().items_center().gap(px(5.)).px(px(9.)).py(px(6.)).rounded(px(5.)).bg(if details { THEME.accent_soft } else { THEME.panel_raised }).text_color(if details { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child("Details").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::Details, cx))))
                                .child(div().id("connection-string-mode").flex().items_center().gap(px(5.)).px(px(9.)).py(px(6.)).rounded(px(5.)).bg(if !details { THEME.accent_soft } else { THEME.panel_raised }).text_color(if !details { THEME.accent } else { THEME.text_muted }).text_size(px(10.)).cursor_pointer().child("Connection string").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx))))))
                            .when(details, |view| view
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Host")).child(editor::input(self.draft.host_editor.clone(), host_focus, false))).child(div().w(px(110.)).flex_none().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Port")).child(editor::input(self.draft.port_editor.clone(), port_focus, false))))
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Username")).child(editor::input(self.draft.username_editor.clone(), username_focus, false))).child(div().flex_1().min_w_0().child(div().text_size(px(10.)).text_color(THEME.text_muted).child("Password")).child(editor::input(self.draft.password_editor.clone(), password_focus, false))))
                                .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child(if kind == DatabaseKind::Redis { "Database index (optional)" } else { "Database" })).child(editor::input(self.draft.database_editor.clone(), database_focus, false))))
                            .when(!details, |view| view.child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(10.)).text_color(THEME.text_muted).child(if kind == DatabaseKind::SQLite { "Database file or connection string" } else { "Connection string" })).child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(editor::input(self.draft.connection_editor.clone(), url_focus, false))).when(kind == DatabaseKind::SQLite, |view| view.child(button("choose-sqlite-file", "Choose file…", ButtonKind::Quiet).flex_none().cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.choose_sqlite_file(cx))))))))
                            .child(div().p(px(9.)).rounded(px(6.)).bg(THEME.canvas).text_size(px(10.)).text_color(THEME.text_muted).child("Profiles stay on disk; passwords stay in the OS keyring.")))))
                .child(div().flex_none().border_t_1().border_color(THEME.border).bg(THEME.panel).px(if self.compact_layout { px(14.) } else { px(24.) }).py(px(12.)).flex().items_center().justify_between().gap(px(12.))
                    .child(div().min_w_0().flex().flex_col().gap(px(4.))
                        .child(div().truncate().text_size(px(10.)).text_color(if self.error.is_some() { THEME.text_muted } else { THEME.success }).child(self.status.clone()))
                        .when_some(self.error.clone(), |view, error| view.child(div().truncate().text_size(px(10.)).text_color(THEME.danger).child(error))))
                    .child(div().flex_none().flex().items_center().gap(px(8.))
                        .child(button("test-connection", if self.testing_connection { "Testing…" } else { "Test connection" }, ButtonKind::Quiet).when(!self.testing_connection, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.test_connection(cx)))))
                        .child(button("save-connection", "Save", ButtonKind::Quiet).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.save_connection(cx))))
                        .child(button("connect", "Connect", ButtonKind::Primary).cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))))))
    }
}
