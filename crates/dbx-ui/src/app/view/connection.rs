use super::super::*;
use gpui_component::{checkbox::Checkbox, switch::Switch};

const VAULT_TEXT_EDITOR_CONTEXT: &str = "DbxTextEditor VaultGate";

/// The new-connection grid, grouped the way people describe their databases.
const KIND_GROUPS: [(&str, &[DatabaseKind]); 4] = [
    (
        "Relational",
        &[
            DatabaseKind::PostgreSQL,
            DatabaseKind::MySQL,
            DatabaseKind::SQLite,
            DatabaseKind::CockroachDB,
            DatabaseKind::DuckDB,
            DatabaseKind::ClickHouse,
        ],
    ),
    (
        "Document & key-value",
        &[DatabaseKind::Redis, DatabaseKind::MongoDB],
    ),
    (
        "Search & streaming",
        &[DatabaseKind::Elasticsearch, DatabaseKind::Kafka],
    ),
    (
        "Cloud",
        &[
            DatabaseKind::BigQuery,
            DatabaseKind::Turso,
            DatabaseKind::CloudflareD1,
        ],
    ),
];

impl DbxApp {
    pub(super) fn render_connection(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.vault_state != Some(VaultState::Unlocked) {
            return self.render_vault_gate(cx).into_any_element();
        }
        let name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let url_focus = self.draft.connection_editor.read(cx).focus_handle();
        let host_focus = self.draft.host_editor.read(cx).focus_handle();
        let port_focus = self.draft.port_editor.read(cx).focus_handle();
        let username_focus = self.draft.username_editor.read(cx).focus_handle();
        let password_focus = self.draft.password_editor.read(cx).focus_handle();
        let database_focus = self.draft.database_editor.read(cx).focus_handle();
        let kind = self.draft.kind;
        let details = self.draft.mode == ConnectionFormMode::Details && kind.supports_details();
        let tags_editor = self.render_connection_tags(cx);
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;
        let editing = selected_profile.is_some();
        let choosing = !editing && self.draft.choosing_kind;
        let kind_picker = choosing.then(|| self.render_kind_picker(cx));
        let transport = self.render_connection_transport(cx);

        if compact_connection_picker_visible(
            self.compact_layout,
            self.compact_connection_form_open,
            saved_connections.len(),
        ) {
            return self
                .render_compact_connection_picker(saved_connections, selected_profile, cx)
                .into_any_element();
        }

        div().flex_1().min_w_0().min_h_0().flex().gap(px(GLASS_INSET)).pr(px(GLASS_INSET)).pb(px(GLASS_INSET))
            .when(!self.compact_layout, |view| view.child(
                glass(div(), RADIUS_GLASS, 8.).w(px(264.)).flex_none().overflow_hidden()
                    .flex().flex_col()
                    .child(div().h(px(48.)).flex_none().pl(px(16.)).pr(px(10.)).flex().items_center().justify_between().border_b_1().border_color(theme().hairline)
                        .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Connections")).when(!saved_connections.is_empty(), |view| view.child(badge(saved_connections.len().to_string(), theme().text_muted))))
                        .child(glass_icon_button("new-connection-from-list", Icon::Add, false).tooltip(tip("New connection")).on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx)))))
                    .child(div().id("saved-connections").flex_1().min_h_0().overflow_y_scroll().p(px(8.)).flex().flex_col().gap(px(2.))
                        .when(saved_connections.is_empty(), |view| view.child(div().p(px(10.)).text_size(px(12.)).text_color(theme().text_muted).child("No saved connections")))
                        .children(saved_connections.into_iter().map(|profile| {
                            let id = profile.id; let selected = selected_profile == Some(id); let choose = profile.clone();
                            div().id(SharedString::from(format!("saved-connection-{id}"))).h(px(50.)).px(px(10.)).rounded(px(RADIUS_PANEL - 2.)).when(selected, |row| row.bg(theme().accent_soft)).when(!selected, |row| row.hover(|style| style.bg(theme().glass_hover))).cursor_pointer().flex().items_center().gap(px(10.))
                                .child(database_logo(profile.kind, if selected { theme().accent } else { theme().text_muted }))
                                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.)).child(div().truncate().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child(profile.name)).child(div().truncate().text_size(px(11.)).text_color(theme().text_muted).child(display_url(&profile.url))))
                                .children(tag_badge(profile.tag.as_ref()))
                                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                                    let click_count = match event {
                                        gpui::ClickEvent::Mouse(mouse) => mouse.up.click_count,
                                        gpui::ClickEvent::Keyboard(_) | gpui::ClickEvent::Touch(_) => 1,
                                    };
                                    match saved_connection_click_action(click_count) {
                                        SavedConnectionClickAction::Select => this.select_saved_connection(choose.clone(), cx),
                                        SavedConnectionClickAction::Open => this.open_saved_connection(choose.clone(), window, cx),
                                    }
                                }))
                        }))),
            ))
            .child(div().flex_1().min_w_0().flex().flex_col().overflow_hidden().rounded(px(RADIUS_PANEL)).border_1().border_color(theme().hairline).bg(theme().canvas).shadow(glass_shadow(10.))
                .child(div().id("connection-form-scroll").flex_1().min_h_0().overflow_y_scroll().p(if self.compact_layout { px(14.) } else { px(24.) }).flex().justify_center().items_start()
                    .child(div().w_full().min_w_0().max_w(px(720.)).flex().flex_col().gap(px(14.))
                        .when(self.compact_layout && !self.saved_connections.is_empty(), |view| view.child(
                            div().flex().child(button("back-to-saved-connections", "Saved connections", ButtonKind::Quiet).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.show_saved_connections(cx))))
                        ))
                        .child(div().id("connection-form-header").flex().items_center().justify_between().gap(px(12.))
                            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if editing { "Edit connection" } else { "New connection" }))
                            // A saved connection keeps its engine; a new one can go back to the grid.
                            .when(!choosing, |view| view.child(div().flex_none().flex().items_center().gap(px(8.))
                                .child(div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(theme().text_muted).child(database_logo(kind, theme().text_muted)).child(kind.to_string()))
                                .when(!editing, |view| view.child(button("change-connection-kind", "Change", ButtonKind::Quiet).h(px(26.)).text_size(px(11.)).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.change_connection_kind(cx))))))))
                        .children(kind_picker)
                        .when(!choosing, |view| view.child(div().bg(theme().canvas).py(px(16.)).flex().flex_col().gap(px(12.))
                            .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child("Connection name")).child(editor::input(self.draft.connection_name_editor.clone(), name_focus, false)))
                            .child(tags_editor)
                            .when(kind.supports_details(), |view| view.child(div().flex().child(segmented_track()
                                .child(div().id("connection-details-mode").flex().items_center().gap(px(5.)).px(px(12.)).h(px(26.)).rounded_full().when(details, |view| view.bg(theme().glass_selected).border_1().border_color(theme().hairline).shadow(glass_shadow(3.))).text_color(if details { theme().text } else { theme().text_muted }).text_size(px(11.)).cursor_pointer().child("Details").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::Details, cx))))
                                .child(div().id("connection-string-mode").flex().items_center().gap(px(5.)).px(px(12.)).h(px(26.)).rounded_full().when(!details, |view| view.bg(theme().glass_selected).border_1().border_color(theme().hairline).shadow(glass_shadow(3.))).text_color(if !details { theme().text } else { theme().text_muted }).text_size(px(11.)).cursor_pointer().child("Connection string").on_click(cx.listener(|this, _, _, cx| this.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx)))))))
                            .when(details, |view| view
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(11.)).text_color(theme().text_muted).child("Host")).child(editor::input(self.draft.host_editor.clone(), host_focus, false))).child(div().w(px(110.)).flex_none().child(div().text_size(px(11.)).text_color(theme().text_muted).child("Port")).child(editor::input(self.draft.port_editor.clone(), port_focus, false))))
                                .child(div().flex().gap(px(8.)).child(div().flex_1().min_w_0().child(div().text_size(px(11.)).text_color(theme().text_muted).child("Username")).child(editor::input(self.draft.username_editor.clone(), username_focus, false))).child(div().flex_1().min_w_0().child(div().text_size(px(11.)).text_color(theme().text_muted).child("Password")).child(editor::input(self.draft.password_editor.clone(), password_focus.clone(), false))))
                                .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child(if kind == DatabaseKind::Redis { "Database index (optional)" } else { "Database" })).child(editor::input(self.draft.database_editor.clone(), database_focus, false))))
                            .when(!kind.connection_help().is_empty(), |view| view.child(div().text_size(px(11.)).text_color(theme().text_muted).child(kind.connection_help())))
                            .when(!details, |view| view.child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child(if kind.is_file() { "Database file or connection string" } else { "Connection string" })).child(div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(editor::input(self.draft.connection_editor.clone(), url_focus, false))).when(kind.is_file(), |view| view.child(button("choose-sqlite-file", "Choose file…", ButtonKind::Quiet).h(px(32.)).flex_none().cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.choose_sqlite_file(cx))))))))
                            .when(matches!(kind,DatabaseKind::BigQuery|DatabaseKind::Turso|DatabaseKind::CloudflareD1), |view| view.child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child("API token")).child(editor::input(self.draft.password_editor.clone(),password_focus,false))))
                            .when(kind.supports_transport(), |view| view.child(transport))))))
                .when(!choosing, |view| view.child(div().flex_none().border_t_1().border_color(theme().border).follow_bottom_corners(RADIUS_PANEL).bg(theme().panel).px(if self.compact_layout { px(14.) } else { px(24.) }).py(px(12.)).flex().items_center().justify_between().gap(px(12.))
                    .child(div().min_w_0().flex_1().when_some(self.error.clone(), |view, error| view.child(div().id("connection-error").truncate().text_size(px(12.)).text_color(theme().danger).tooltip(tip(error.clone())).child(error))))
                    .child(div().flex_none().flex().items_center().gap(px(8.))
                        .child(button("test-connection", if self.testing_connection { "Testing…" } else if self.credential_hydrating { "Loading password…" } else { "Test connection" }, ButtonKind::Quiet).when(!self.testing_connection && !self.credential_hydrating, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.test_connection(cx)))))
                        .child(button("save-connection", if self.saving_connection { "Saving…" } else { "Save" }, ButtonKind::Quiet).when(!self.vault_busy && !self.saving_connection, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.save_connection(cx)))))
                        .child(button("connect", if self.credential_hydrating { "Loading password…" } else { "Connect" }, ButtonKind::Primary).when(!self.credential_hydrating, |button| button.cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))))))))).into_any_element()
    }

    /// The first step of a new connection: paste a URL to detect its type, or
    /// pick one from the grid.
    fn render_kind_picker(&self, cx: &mut Context<Self>) -> Div {
        let url_focus = self.draft.import_url_editor.read(cx).focus_handle();
        let detected = self.import_url_kind(cx);
        div()
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme().text_muted)
                            .child("Paste a connection URL"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().child(editor::input(
                                self.draft.import_url_editor.clone(),
                                url_focus,
                                false,
                            )))
                            .child(
                                button(
                                    "continue-with-url",
                                    match detected {
                                        Some(kind) => format!("Continue as {kind}"),
                                        None => "Continue".into(),
                                    },
                                    if detected.is_some() {
                                        ButtonKind::Primary
                                    } else {
                                        ButtonKind::Quiet
                                    },
                                )
                                .h(px(32.))
                                .flex_none()
                                .disabled(detected.is_none())
                                .when(
                                    detected.is_some(),
                                    |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.continue_with_import_url(cx),
                                        ))
                                    },
                                ),
                            ),
                    ),
            )
            .children(KIND_GROUPS.into_iter().map(|(title, kinds)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme().text_muted)
                            .child(title),
                    )
                    .child(div().flex().flex_wrap().gap(px(8.)).children(
                        kinds.iter().copied().map(|kind| {
                            div()
                                .id(SharedString::from(format!("engine-{kind}")))
                                .w(px(120.))
                                .h(px(72.))
                                .rounded(px(RADIUS_PANEL))
                                .border_1()
                                .border_color(theme().hairline)
                                .bg(theme().glass_hover)
                                .hover(|style| {
                                    style
                                        .bg(theme().accent_soft)
                                        .border_color(theme().accent.alpha(0.45))
                                })
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(8.))
                                .cursor_pointer()
                                .child(database_logo(kind, theme().text).size(px(22.)))
                                .child(div().text_size(px(12.)).child(kind.to_string()))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.select_kind(kind, cx)),
                                )
                        }),
                    ))
            }))
    }

    /// Tags are picked here and managed in Settings, so the form stays short.
    fn render_connection_tags(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme().text_muted)
                    .child("Tags"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .children(self.connection_tags.iter().map(|tag| {
                        let selected = self
                            .draft
                            .tag
                            .as_ref()
                            .is_some_and(|item| item.id == tag.id);
                        let choose = tag.clone();
                        div()
                            .id(SharedString::from(format!("tag-{}", tag.id)))
                            .px(px(10.))
                            .h(px(26.))
                            .rounded_full()
                            .border_1()
                            .border_color(gpui::rgb(tag.color))
                            .when(selected, |view| view.bg(gpui::rgb(tag.color).alpha(0.15)))
                            .text_color(gpui::rgb(tag.color))
                            .text_size(px(11.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .child(format!("{}{}", if selected { "✓ " } else { "" }, tag.name))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_tag(choose.clone(), cx)
                            }))
                    }))
                    .child(
                        button("manage-tags", "Manage tags…", ButtonKind::Quiet)
                            .h(px(26.))
                            .text_size(px(11.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    ),
            )
    }

    fn render_connection_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        let transport = &self.draft.transport;
        let socket_enabled = transport.socket_enabled;
        let ssh_enabled = transport.ssh_enabled;
        let input = |label: &'static str, entity: Entity<TextEditor>| {
            let focus = entity.read(cx).focus_handle();
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme().text_muted)
                        .child(label),
                )
                .child(editor::input(entity, focus, false))
                .into_any_element()
        };
        let socket = input(
            if self.draft.kind == DatabaseKind::PostgreSQL {
                "Socket directory (e.g. /var/run/postgresql)"
            } else {
                "Socket file (e.g. /tmp/mysql.sock)"
            },
            transport.socket_editor.clone(),
        );
        let host = input("SSH host", transport.ssh_host_editor.clone());
        let port = input("SSH port", transport.ssh_port_editor.clone());
        let user = input("SSH username", transport.ssh_user_editor.clone());
        let key = input(
            "Private key file (optional; leave blank to use SSH agent/config)",
            transport.ssh_key_editor.clone(),
        );
        div().debug_selector(|| "connection-transport-form".into()).border_t_1().border_color(theme().hairline).pt(px(12.))
            .flex().flex_col().gap(px(10.))
            .child(Switch::new("connection-unix-socket").label("Use Unix socket")
                .checked(socket_enabled).with_size(Size::Small)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.draft.transport.socket_enabled = *checked;
                    this.error = None;
                    cx.notify();
                })))
            .when(socket_enabled, |view| view.child(socket).child(
                div().text_size(px(11.)).text_color(theme().text_muted)
                    .child(if ssh_enabled { "Socket on the SSH server. PostgreSQL uses the database port to select its socket." }
                        else { "Local socket. Host is ignored; PostgreSQL uses the database port to select its socket." })))
            .child(Switch::new("connection-ssh-tunnel").label("Connect through SSH tunnel")
                .checked(ssh_enabled).with_size(Size::Small)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.draft.transport.ssh_enabled = *checked;
                    this.error = None;
                    cx.notify();
                })))
            .when(ssh_enabled, |view| view
                .child(div().flex().gap(px(8.)).child(host).child(div().w(px(110.)).child(port)))
                .child(user)
                .child(div().flex().items_end().gap(px(8.)).child(key)
                    .child(button("choose-ssh-key", "Choose key…", ButtonKind::Quiet)
                        .cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.choose_ssh_key(cx)))))
                .child(div().text_size(px(11.)).text_color(theme().text_muted)
                    .child("Database host is resolved from the SSH server. Uses your SSH agent or key; load encrypted keys into the agent. Verify a new SSH host in a terminal first.")))
            .into_any_element()
    }

    fn render_compact_connection_picker(
        &mut self,
        saved_connections: Vec<SavedConnection>,
        selected_profile: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let profile_count = saved_connections.len();
        let has_selected_profile = selected_profile.is_some();
        div()
            .id("compact-connection-picker")
            .flex_1()
            .min_h_0()
            .flex()
            .justify_center()
            .p(px(14.))
            .pt(px(4.))
            .child(
                div()
                    .w_full()
                    .max_w(px(720.))
                    .h_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Connections"),
                    )
                    .child(
                        glass(div(), RADIUS_GLASS, 8.)
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(px(44.))
                                    .flex_none()
                                    .px(px(12.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .border_b_1()
                                    .border_color(theme().hairline)
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Saved connections"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme().text_muted)
                                            .child(profile_count.to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("saved-connections")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p(px(8.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .children(saved_connections.into_iter().map(|profile| {
                                        let id = profile.id;
                                        let selected = selected_profile == Some(id);
                                        let choose = profile.clone();
                                        div()
                                            .id(SharedString::from(format!(
                                                "saved-connection-{id}"
                                            )))
                                            .h(px(50.))
                                            .flex_none()
                                            .px(px(10.))
                                            .rounded(px(RADIUS_PANEL - 2.))
                                            .when(selected, |row| row.bg(theme().accent_soft))
                                            .when(!selected, |row| {
                                                row.hover(|style| style.bg(theme().glass_hover))
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(database_logo(
                                                profile.kind,
                                                if selected {
                                                    theme().accent
                                                } else {
                                                    theme().text_muted
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(profile.name),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_size(px(11.))
                                                            .text_color(theme().text_muted)
                                                            .child(display_url(&profile.url)),
                                                    ),
                                            )
                                            .children(tag_badge(profile.tag.as_ref()))
                                            .on_click(cx.listener(
                                                move |this,
                                                      event: &gpui::ClickEvent,
                                                      window,
                                                      cx| {
                                                    let click_count = match event {
                                                        gpui::ClickEvent::Mouse(mouse) => {
                                                            mouse.up.click_count
                                                        }
                                                        gpui::ClickEvent::Keyboard(_)
                                                        | gpui::ClickEvent::Touch(_) => 1,
                                                    };
                                                    match saved_connection_click_action(click_count)
                                                    {
                                                        SavedConnectionClickAction::Select => this
                                                            .select_saved_connection_in_compact_picker(
                                                                choose.clone(),
                                                                cx,
                                                            ),
                                                        SavedConnectionClickAction::Open => this
                                                            .open_saved_connection(
                                                                choose.clone(),
                                                                window,
                                                                cx,
                                                            ),
                                                    }
                                                },
                                            ))
                                    })),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .border_t_1()
                                    .border_color(theme().hairline)
                                    .p(px(12.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(8.))
                                            .when(has_selected_profile, |view| {
                                                view.child(
                                                    button(
                                                        "compact-edit-connection",
                                                        "Edit selected",
                                                        ButtonKind::Quiet,
                                                    )
                                                    .flex_1()
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.show_selected_connection_form(cx)
                                                    })),
                                                )
                                            })
                                            .child(
                                                button(
                                                    "compact-new-connection",
                                                    "New connection",
                                                    ButtonKind::Primary,
                                                )
                                                .flex_1()
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.begin_new_connection(cx)
                                                })),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    fn render_vault_gate(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.vault_state;
        let creating = state == Some(VaultState::Uninitialized);
        let unavailable = state.is_none();
        let passphrase_focus = self.vault_editors.passphrase_editor.read(cx).focus_handle();
        let confirmation_focus = self
            .vault_editors
            .confirmation_editor
            .read(cx)
            .focus_handle();
        let remember_device = Checkbox::new("vault-remember-device")
            .label("Unlock automatically on this device")
            .checked(self.remember_device)
            .with_size(Size::Small)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_remember_device(cx)));
        let submit_label = match (self.vault_busy, creating) {
            (true, true) => "Creating…",
            (true, false) => "Unlocking…",
            (false, true) => "Create vault",
            (false, false) => "Unlock",
        };
        let submit = button("submit-vault-passphrase", submit_label, ButtonKind::Primary).when(
            !self.vault_busy,
            |button| {
                button.cursor_pointer().on_click(
                    cx.listener(move |this, _, _, cx| this.submit_vault_passphrase(creating, cx)),
                )
            },
        );
        div().key_context("VaultGate")
            .on_action(cx.listener(|_, _: &VaultFocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &VaultFocusPrevious, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(move |this, _: &SubmitVault, _, cx| {
                if !this.vault_busy {
                    this.submit_vault_passphrase(creating, cx);
                }
            }))
            .flex_1().min_h_0().flex().items_center().justify_center().p(px(20.))
            .child(glass_raised(div(), RADIUS_GLASS + 4.).w_full().max_w(px(420.)).p(px(24.)).flex().flex_col().gap(px(14.))
                .child(img(self.logo.clone()).id("vault-logo").size(px(40.)))
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if unavailable { "DBX Vault unavailable" } else if creating { "Create DBX Vault" } else { "Unlock DBX Vault" }))
                .when(unavailable, |view| view.child(div().text_size(px(11.)).text_color(theme().text_muted).child("Saved connection passwords can’t be read on this device.")))
                .when(creating, |view| view.child(div().p(px(10.)).rounded(px(RADIUS_PANEL - 2.)).bg(theme().accent_soft).text_size(px(11.)).text_color(theme().text_muted).child("If this passphrase is lost, saved passwords cannot be recovered.")))
                .when(!unavailable, |view| view
                    .child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child("Passphrase (12 characters minimum)")).child(editor::input_with_key_context(self.vault_editors.passphrase_editor.clone(), passphrase_focus, false, VAULT_TEXT_EDITOR_CONTEXT)))
                    .when(creating, |view| view.child(div().flex().flex_col().gap(px(5.)).child(div().text_size(px(11.)).text_color(theme().text_muted).child("Confirm passphrase")).child(editor::input_with_key_context(self.vault_editors.confirmation_editor.clone(), confirmation_focus, false, VAULT_TEXT_EDITOR_CONTEXT))))
                    .child(div().flex().items_center().justify_between().gap(px(12.))
                        .child(remember_device)
                        .child(submit)))
                .when_some(self.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(theme().danger).child(error))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_groups_list_every_database_once() {
        let mut listed: Vec<_> = KIND_GROUPS
            .iter()
            .flat_map(|(_, kinds)| kinds.iter().copied())
            .collect();
        assert_eq!(listed.len(), DatabaseKind::ALL.len());
        listed.sort_by_key(|kind| kind.to_string());
        listed.dedup();
        assert_eq!(listed.len(), DatabaseKind::ALL.len());
    }
}
