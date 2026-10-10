use super::*;
use crate::agents::{AgentCli, AgentConfig, AgentPreferences, GeneratedQuery};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use std::time::Duration;

/// The prompt field shares the editor's dispatch depth so Enter and Escape
/// reach the agent before the editor's own bindings.
const AGENT_PROMPT_CONTEXT: &str = "DbxTextEditor DbxQueryAgent";
const SAVE_DELAY: Duration = Duration::from_millis(500);

struct SetupField {
    value: Entity<String>,
    editor: Entity<TextEditor>,
}

impl SetupField {
    fn new(text: String, window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        Self::with_multiline(text, false, window, cx)
    }
    fn with_multiline(
        text: String,
        multiline: bool,
        window: &mut Window,
        cx: &mut Context<DbxApp>,
    ) -> Self {
        let value = cx.new(|_| text);
        let editor = cx.new(|cx| TextEditor::new(value.clone(), multiline, window, cx));
        Self { value, editor }
    }
    fn focus_handle(&self, cx: &Context<DbxApp>) -> FocusHandle {
        self.editor.read(cx).focus_handle()
    }
    fn input(&self, cx: &Context<DbxApp>) -> impl IntoElement {
        editor::input(self.editor.clone(), self.focus_handle(cx), false)
    }
    fn text(&self, cx: &Context<DbxApp>) -> String {
        self.value.read(cx).trim().to_owned()
    }
    fn set(&self, text: String, cx: &mut Context<DbxApp>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text(text, cx));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CliStatus {
    Checking,
    Ready(String),
    Failed(String),
}

pub(super) struct AgentSetup {
    pub preferences: AgentPreferences,
    executable: SetupField,
    model: SetupField,
    provider: SetupField,
    status: Option<CliStatus>,
    check_generation: u64,
    save_generation: u64,
    recheck_pending: bool,
    advanced_open: bool,
    save_pending: bool,
    pub save_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl AgentSetup {
    pub fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let preferences = SettingsStore::new()
            .and_then(|store| store.load())
            .map(|settings| settings.agents)
            .unwrap_or_default();
        let config = configuration(&preferences, preferences.default_cli);
        let executable = SetupField::new(config.executable, window, cx);
        let model = SetupField::new(config.model, window, cx);
        let provider = SetupField::new(config.provider, window, cx);
        // Edits save themselves; there is no separate Save step to forget.
        let _subscriptions = [&executable, &model, &provider]
            .into_iter()
            .map(|field| {
                cx.observe(&field.value, |this: &mut DbxApp, _, cx| {
                    this.agent_fields_changed(cx)
                })
            })
            .collect();
        Self {
            preferences,
            executable,
            model,
            provider,
            status: None,
            check_generation: 0,
            save_generation: 0,
            recheck_pending: false,
            advanced_open: false,
            save_pending: false,
            save_error: None,
            _subscriptions,
        }
    }

    fn fields(&self, cx: &Context<DbxApp>) -> AgentConfig {
        let cli = self.preferences.default_cli;
        let executable = self.executable.text(cx);
        AgentConfig {
            executable: if executable.is_empty() {
                cli.executable().into()
            } else {
                executable
            },
            model: self.model.text(cx),
            provider: self.provider.text(cx),
        }
    }
}

/// A CLI's saved configuration, with its conventional executable name filled in.
fn configuration(preferences: &AgentPreferences, cli: AgentCli) -> AgentConfig {
    let mut config = preferences
        .configurations
        .get(&cli)
        .cloned()
        .unwrap_or_default();
    if config.executable.trim().is_empty() {
        config.executable = cli.executable().into();
    }
    config
}

pub(super) struct AgentQuery {
    pub open: bool,
    prompt: SetupField,
    pub result: Option<GeneratedQuery>,
    error: Option<String>,
    busy: bool,
    generation: u64,
    abort: AbortOnDrop,
    database: Option<String>,
    generated_by: Option<String>,
    _subscription: Subscription,
}

impl AgentQuery {
    pub fn new(window: &mut Window, cx: &mut Context<DbxApp>) -> Self {
        let prompt = SetupField::with_multiline(String::new(), true, window, cx);
        let subscription = cx.observe(&prompt.value, |_, _, cx| cx.notify());
        Self {
            open: false,
            prompt,
            result: None,
            error: None,
            busy: false,
            generation: 0,
            abort: AbortOnDrop::default(),
            database: None,
            generated_by: None,
            _subscription: subscription,
        }
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.abort.cancel();
        self.busy = false;
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }
}

impl DbxApp {
    pub(super) fn check_agent_cli(&mut self, cx: &mut Context<Self>) {
        // The check runs on the tokio runtime, which GPUI's deterministic test
        // scheduler rejects; adapter launches have their own process tests.
        if cfg!(test) {
            return;
        }
        let setup = &mut self.agent_setup;
        let cli = setup.preferences.default_cli;
        let executable = setup.executable.text(cx);
        setup.check_generation = setup.check_generation.wrapping_add(1);
        setup.recheck_pending = false;
        let generation = setup.check_generation;
        setup.status = Some(CliStatus::Checking);
        let task = self
            .runtime
            .spawn(crate::agents::check_cli(cli, executable));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.agent_setup.check_generation != generation {
                    return;
                }
                this.agent_setup.status = Some(match result {
                    Ok(Ok(version)) => CliStatus::Ready(version),
                    Ok(Err(error)) => CliStatus::Failed(error),
                    Err(_) => CliStatus::Failed("The check was interrupted.".into()),
                });
                if matches!(this.agent_setup.status, Some(CliStatus::Failed(_))) {
                    this.agent_setup.advanced_open = true;
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Make `cli` the agent used for generation and show its configuration.
    fn select_agent(&mut self, cli: AgentCli, cx: &mut Context<Self>) {
        if self.agent_setup.preferences.default_cli == cli {
            return;
        }
        // Flush the outgoing fields even if their change observer has not run yet.
        self.commit_agent_fields(cx);
        let setup = &mut self.agent_setup;
        let changed = setup.preferences.default_cli != cli;
        setup.preferences.default_cli = cli;
        let config = configuration(&setup.preferences, cli);
        setup.executable.set(config.executable, cx);
        setup.model.set(config.model, cx);
        setup.provider.set(config.provider, cx);
        if changed {
            self.persist_settings(cx);
        }
        self.check_agent_cli(cx);
    }

    fn agent_fields_changed(&mut self, cx: &mut Context<Self>) {
        let setup = &mut self.agent_setup;
        let cli = setup.preferences.default_cli;
        let config = setup.fields(cx);
        let saved = configuration(&setup.preferences, cli);
        // Switching agents rewrites every field; those echoes are not edits.
        if config == saved {
            return;
        }
        setup.recheck_pending |= config.executable != saved.executable;
        if setup.recheck_pending {
            setup.check_generation = setup.check_generation.wrapping_add(1);
            setup.status = None;
        }
        setup.save_pending = true;
        setup.preferences.configurations.insert(cli, config);
        setup.save_generation = setup.save_generation.wrapping_add(1);
        let generation = setup.save_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            this.update(cx, |this, cx| {
                if this.agent_setup.save_generation == generation {
                    this.commit_agent_fields(cx);
                }
            })
        })
        .detach();
    }

    fn commit_agent_fields(&mut self, cx: &mut Context<Self>) {
        self.agent_setup.save_generation = self.agent_setup.save_generation.wrapping_add(1);
        self.agent_setup.save_pending = false;
        let cli = self.agent_setup.preferences.default_cli;
        let config = self.agent_setup.fields(cx);
        self.agent_setup
            .preferences
            .configurations
            .insert(cli, config);
        self.persist_settings(cx);
        if self.agent_setup.recheck_pending {
            self.check_agent_cli(cx);
        }
        cx.notify();
    }

    fn open_agent_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = SettingsSection::QueryAgent;
        self.open_settings(cx);
    }

    fn agent_query_mut(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
    ) -> Option<&mut AgentQuery> {
        let tab = self
            .session_mut(session_id)?
            .secondary_tabs
            .iter_mut()
            .find(|t| t.id == tab_id)?;
        match &mut tab.kind {
            SecondaryTabKind::Query(query) => Some(&mut query.agent),
            _ => None,
        }
    }

    fn agent_query(&self, session_id: SessionId, tab_id: SecondaryTabId) -> Option<&AgentQuery> {
        let tab = self
            .session(session_id)?
            .secondary_tabs
            .iter()
            .find(|t| t.id == tab_id)?;
        match &tab.kind {
            SecondaryTabKind::Query(query) => Some(&query.agent),
            _ => None,
        }
    }

    pub(super) fn render_agent_settings(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let selected = self.agent_setup.preferences.default_cli;
        let choices = AgentCli::ALL.into_iter().map(|cli| {
            segment(cli.label(), cli == selected)
                .id(SharedString::from(format!("agent-choice-{cli:?}")))
                .pressable()
                .debug_selector(move || format!("agent-choice-{cli:?}"))
                .on_click(cx.listener(move |this, _, _, cx| this.select_agent(cli, cx)))
        });
        let (status_title, detail, color) = match &self.agent_setup.status {
            None => ("Not checked", None, theme().text_muted),
            Some(CliStatus::Checking) => ("Checking…", None, theme().text_muted),
            Some(CliStatus::Ready(version)) => {
                ("Installed", Some(version.clone()), theme().success)
            }
            Some(CliStatus::Failed(error)) => (
                "Unavailable",
                Some(format!(
                    "{error} Install and sign in to {} in a terminal.",
                    selected.label()
                )),
                theme().danger,
            ),
        };
        let checking = matches!(self.agent_setup.status, Some(CliStatus::Checking));
        let has_provider = matches!(selected, AgentCli::Codex | AgentCli::OpenCode);
        let advanced = self.agent_setup.advanced_open;
        let provider_help = if selected == AgentCli::OpenCode {
            "OpenCode provider ID, or enter provider/model above"
        } else {
            "A model_provider ID from your Codex configuration"
        };
        let field = |input| div().w(px(260.)).child(input);
        let mut advanced_rows = vec![
            settings_row(
                "Advanced",
                None,
                button(
                    "agent-advanced-settings",
                    if advanced { "Hide" } else { "Show" },
                    ButtonKind::Quiet,
                )
                .debug_selector(|| "agent-advanced-settings".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent_setup.advanced_open = !this.agent_setup.advanced_open;
                    cx.notify();
                })),
            )
            .into_any_element(),
        ];
        if advanced {
            advanced_rows.push(
                settings_row(
                    "Executable",
                    Some("Command name or full path".into()),
                    field(self.agent_setup.executable.input(cx)),
                )
                .into_any_element(),
            );
            if has_provider {
                advanced_rows.push(
                    settings_row(
                        "Provider",
                        Some(provider_help.into()),
                        field(self.agent_setup.provider.input(cx)),
                    )
                    .into_any_element(),
                );
            }
        }
        div()
            .id("agents-settings")
            .debug_selector(|| "agents-settings".into())
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(settings_group([
                        settings_row("CLI", None, segmented_track().children(choices))
                            .into_any_element(),
                        settings_row(
                            div()
                                .id("agent-cli-status")
                                .debug_selector(|| "agent-cli-status".into())
                                .text_color(color)
                                .child(status_title),
                            detail.map(SharedString::from),
                            button("recheck-agent-cli", "Check again", ButtonKind::Quiet)
                                .disabled(checking)
                                .loading(checking)
                                .on_click(cx.listener(|this, _, _, cx| this.check_agent_cli(cx))),
                        )
                        .into_any_element(),
                        settings_row(
                            "Model",
                            Some("Blank uses the CLI’s default".into()),
                            field(self.agent_setup.model.input(cx)),
                        )
                        .into_any_element(),
                    ]))
                    .child(
                        div()
                            .px(px(16.))
                            .text_size(px(11.))
                            .text_color(theme().text_muted)
                            .child(
                                "Your description and schema are sent through the CLI to its \
                                 provider. Row data and credentials are not.",
                            ),
                    ),
            )
            .child(settings_group(advanced_rows))
            .when_some(self.agent_setup.save_error.clone(), |view, error| {
                view.child(settings_group([settings_row(
                    div()
                        .text_color(theme().danger)
                        .child("Changes could not be saved"),
                    Some(error.into()),
                    button("retry-agent-save", "Retry", ButtonKind::Quiet)
                        .on_click(cx.listener(|this, _, _, cx| this.commit_agent_fields(cx))),
                )
                .into_any_element()]))
            })
    }

    pub(super) fn agent_panel_open(&self, session_id: SessionId, tab_id: SecondaryTabId) -> bool {
        self.agent_query(session_id, tab_id)
            .is_some_and(|agent| agent.open)
    }

    pub(super) fn toggle_agent_panel(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = !self.agent_panel_open(session_id, tab_id);
        self.set_agent_panel_open(session_id, tab_id, open, window, cx);
    }

    fn set_agent_panel_open(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session.secondary_tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let SecondaryTabKind::Query(query) = &mut tab.kind else {
            return;
        };
        query.agent.open = open;
        let focus = if open {
            query.agent.prompt.editor.read(cx).focus_handle()
        } else {
            query.query_editor.read(cx).focus_handle()
        };
        focus.focus(window, cx);
        if open && self.agent_setup.status.is_none() {
            self.check_agent_cli(cx);
        }
        cx.notify();
    }

    /// Escape stops a running generation first, then closes the panel.
    fn dismiss_agent_panel(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.agent_query_mut(session_id, tab_id) else {
            return;
        };
        if agent.busy {
            agent.cancel();
            cx.notify();
        } else {
            self.set_agent_panel_open(session_id, tab_id, false, window, cx);
        }
    }

    /// Replace the query document with the generated query, optionally running it.
    fn use_generated_query(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        if session.busy {
            return;
        }
        let database = session.current_database.clone();
        let run_all = session.kind.is_sql();
        let Some(tab) = session.secondary_tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let SecondaryTabKind::Query(query) = &mut tab.kind else {
            return;
        };
        if query.busy || query.agent.busy || query.agent.database != database {
            return;
        }
        let Some(result) = query.agent.result.take() else {
            return;
        };
        query.agent.error = None;
        // set_text records undo history, so replacing the document is reversible.
        query
            .query_editor
            .update(cx, |editor, cx| editor.set_text(result.query, cx));
        self.set_agent_panel_open(session_id, tab_id, false, window, cx);
        if run {
            self.request_run_query_for(session_id, run_all, window, cx);
        }
    }

    pub(super) fn render_agent_panel(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let cli = self.agent_setup.preferences.default_cli;
        let model = configuration(&self.agent_setup.preferences, cli).model;
        let session = self.session(session_id)?;
        let context_label = format!(
            "{} · {}",
            session.kind,
            session.current_database.as_deref().unwrap_or(&session.name)
        );
        let connected = session.engine.is_some() && !session.busy;
        let can_apply = !session.busy && session.secondary_tabs.iter().any(|tab| {
            tab.id == tab_id && matches!(&tab.kind, SecondaryTabKind::Query(query) if !query.busy && !query.agent.busy && query.agent.database == session.current_database)
        });
        let examples = session.kind.is_sql().then(|| {
            let table = session
                .tables
                .first()
                .map(|table| table.name.as_str())
                .unwrap_or("a table");
            [
                format!("Show the first 20 rows from {table}"),
                format!("Count the rows in {table}"),
            ]
        });
        let unavailable = matches!(self.agent_setup.status, Some(CliStatus::Failed(_)));
        let app = cx.entity().downgrade();
        let agent = self.agent_query(session_id, tab_id)?;
        if !agent.open {
            return None;
        }
        let busy = agent.busy;
        let result = agent.result.clone();
        let error = agent.error.clone();
        let prompt_empty = agent.prompt.value.read(cx).trim().is_empty();
        let prompt = agent.prompt.editor.clone();
        let prompt_focus = prompt.read(cx).focus_handle();
        let generated_by = agent.generated_by.clone();
        let agent_label = if model.is_empty() {
            cli.label().to_string()
        } else {
            format!("{} · {model}", cli.label())
        };
        let agent_menu = Button::new("query-agent-picker")
            .with_size(Size::XSmall)
            .ghost()
            .label(agent_label)
            .disabled(busy)
            .tooltip("Choose query assistant")
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                for choice in AgentCli::ALL {
                    let app = app.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.label())
                            .checked(choice == cli)
                            .on_click(move |_, _, cx| {
                                let _ = app.update(cx, |this, cx| this.select_agent(choice, cx));
                            }),
                    );
                }
                let app = app.clone();
                menu.separator()
                    .item(PopupMenuItem::new("Query assistant settings…").on_click(
                        move |_, _, cx| {
                            let _ = app.update(cx, |this, cx| this.open_agent_settings(cx));
                        },
                    ))
            });
        let action =
            if busy {
                button("cancel-agent-query", "Stop", ButtonKind::Quiet)
                    .tooltip("Stop generation (Esc)")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(agent) = this.agent_query_mut(session_id, tab_id) {
                            agent.cancel();
                        }
                        cx.notify();
                    }))
            } else {
                button(
                    "generate-agent-query",
                    if result.is_some() {
                        "Regenerate"
                    } else {
                        "Generate query"
                    },
                    ButtonKind::Primary,
                )
                .debug_selector(|| "generate-agent-query".into())
                .disabled(prompt_empty || unavailable || !connected)
                .when(prompt_empty || unavailable || !connected, |button| {
                    button.opacity(0.45)
                })
                .tooltip("Generate query (Enter). Shift+Enter adds a line.")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.generate_agent_query(session_id, tab_id, cx)
                }))
            };
        Some(
            div()
                .id("query-agent-panel")
                .debug_selector(|| "query-agent-panel".into())
                .flex_none()
                .max_h(px(if self.compact_layout { 360. } else { 460. }))
                .overflow_y_scroll()
                .px(px(12.))
                .py(px(10.))
                .border_b_1()
                .border_color(theme().border)
                .bg(theme().panel)
                .flex()
                .flex_col()
                .gap(px(10.))
                .on_action(cx.listener(move |this, _: &SubmitQueryAgent, _, cx| {
                    this.generate_agent_query(session_id, tab_id, cx);
                    cx.stop_propagation();
                }))
                .on_action(cx.listener(move |this, _: &DismissQueryAgent, window, cx| {
                    this.dismiss_agent_panel(session_id, tab_id, window, cx);
                    cx.stop_propagation();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon(Icon::Sparkles, theme().accent))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Query assistant"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(theme().text_muted)
                                .child(context_label),
                        )
                        .child(
                            Button::new("close-agent-panel")
                                .with_size(Size::XSmall)
                                .compact()
                                .ghost()
                                .tooltip("Close assistant")
                                .child(icon(Icon::Close, theme().text_muted))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(agent) = this.agent_query_mut(session_id, tab_id) {
                                        agent.cancel();
                                    }
                                    this.set_agent_panel_open(
                                        session_id, tab_id, false, window, cx,
                                    );
                                })),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .child(
                            editor::input_with_key_context(
                                prompt,
                                prompt_focus,
                                true,
                                AGENT_PROMPT_CONTEXT,
                            )
                            .h(px(76.)),
                        )
                        .when(prompt_empty, |view| {
                            view.child(div().absolute().top(px(10.)).left(px(11.))
                    .text_size(px(12.)).text_color(theme().text_muted)
                    .child("Describe the data you want, including filters and sorting…"))
                        }),
                )
                .when(
                    prompt_empty && !busy && result.is_none() && !unavailable,
                    |view| {
                        view.when_some(examples, |view, examples| {
                            view.child(div().flex().flex_wrap().gap(px(6.)).children(
                                examples.into_iter().enumerate().map(|(index, example)| {
                                    button(
                                        SharedString::from(format!("agent-example-{index}")),
                                        example.clone(),
                                        ButtonKind::Quiet,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            if let Some(agent) =
                                                this.agent_query_mut(session_id, tab_id)
                                            {
                                                agent.prompt.set(example.clone(), cx);
                                                agent.prompt.focus_handle(cx).focus(window, cx);
                                            }
                                            cx.notify();
                                        },
                                    ))
                                }),
                            ))
                        })
                    },
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(agent_menu)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(140.))
                                .text_size(px(10.))
                                .text_color(theme().text_muted)
                                .child(if busy {
                                    "Reading schema and generating…"
                                } else {
                                    "Schema shared with your CLI · no row data"
                                }),
                        )
                        .child(action),
                )
                .when(unavailable && !busy, |view| {
                    view.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(180.))
                                    .text_size(px(11.))
                                    .text_color(theme().warning)
                                    .child(format!(
                                        "{} is unavailable. Check its executable in settings.",
                                        cli.label()
                                    )),
                            )
                            .child(
                                button("query-agent-setup", "Open settings", ButtonKind::Quiet)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_agent_settings(cx)),
                                    ),
                            ),
                    )
                })
                .when_some(error, |view, error| {
                    view.child(
                        div()
                            .id("agent-query-error")
                            .p(px(10.))
                            .rounded(px(RADIUS_CONTROL))
                            .border_1()
                            .border_color(theme().danger.alpha(0.3))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme().danger)
                                    .child("Could not generate a query"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme().text_muted)
                                    .child(error),
                            )
                            .child(
                                button(
                                    "agent-error-settings",
                                    "Check assistant settings",
                                    ButtonKind::Quiet,
                                )
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.open_agent_settings(cx)),
                                ),
                            ),
                    )
                })
                .when_some(result, |view, result| {
                    view.child(
                        div()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .rounded(px(RADIUS_CONTROL))
                            .border_1()
                            .border_color(theme().border)
                            .bg(theme().canvas)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .px(px(10.))
                                    .py(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Generated query"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(10.))
                                            .text_color(theme().text_muted)
                                            .child(generated_by.unwrap_or_default()),
                                    )
                                    .child(
                                        button("copy-generated-query", "Copy", ButtonKind::Quiet)
                                            .on_click(cx.listener({
                                                let text = result.query.clone();
                                                move |_, _, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(text.clone()),
                                                    )
                                                }
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .id("generated-query-preview")
                                    .debug_selector(|| "generated-query-preview".into())
                                    .max_h(px(140.))
                                    .overflow_y_scroll()
                                    .px(px(10.))
                                    .py(px(8.))
                                    .font_family("monospace")
                                    .text_size(px(12.))
                                    .text_color(theme().text)
                                    .child(result.query),
                            )
                            .child(
                                div()
                                    .px(px(10.))
                                    .pb(px(8.))
                                    .text_size(px(11.))
                                    .text_color(theme().text_muted)
                                    .child(result.explanation),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(10.))
                                    .py(px(8.))
                                    .border_t_1()
                                    .border_color(theme().hairline)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(120.))
                                            .text_size(px(10.))
                                            .text_color(theme().text_muted)
                                            .child("Replaces the current editor query"),
                                    )
                                    .child(
                                        button(
                                            "discard-generated-query",
                                            "Discard",
                                            ButtonKind::Quiet,
                                        )
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if let Some(agent) =
                                                    this.agent_query_mut(session_id, tab_id)
                                                {
                                                    agent.result = None;
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        button(
                                            "run-generated-query",
                                            "Insert & run",
                                            ButtonKind::Quiet,
                                        )
                                        .debug_selector(|| "run-generated-query".into())
                                        .disabled(!can_apply)
                                        .when(!can_apply, |button| button.opacity(0.45))
                                        .tooltip("Replace the editor query and execute it")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.use_generated_query(
                                                    session_id, tab_id, true, window, cx,
                                                )
                                            }),
                                        ),
                                    )
                                    .child(
                                        button(
                                            "use-generated-query",
                                            "Insert query",
                                            ButtonKind::Primary,
                                        )
                                        .debug_selector(|| "use-generated-query".into())
                                        .disabled(!can_apply)
                                        .when(!can_apply, |button| button.opacity(0.45))
                                        .tooltip("Replace the editor query for review")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.use_generated_query(
                                                    session_id, tab_id, false, window, cx,
                                                )
                                            }),
                                        ),
                                    ),
                            ),
                    )
                }),
        )
    }

    fn generate_agent_query(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.agent_setup.status, Some(CliStatus::Failed(_))) {
            return;
        }
        if self
            .agent_query(session_id, tab_id)
            .is_none_or(|agent| agent.busy || agent.prompt.text(cx).is_empty())
        {
            return;
        }
        self.commit_agent_fields(cx);
        let cli = self.agent_setup.preferences.default_cli;
        let config = configuration(&self.agent_setup.preferences, cli);
        let generated_by = if config.model.is_empty() {
            cli.label().to_owned()
        } else {
            format!("{} · {}", cli.label(), config.model)
        };
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy {
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let database = session.current_database.clone();
        let kind = session.kind;
        // Non-relational context uses metadata already fetched by the explorer. It never samples rows.
        let metadata = serde_json::json!({ "entities": session.tables, "known_columns": session.completion_columns });
        let Some(agent) = self.agent_query_mut(session_id, tab_id) else {
            return;
        };
        let request = agent.prompt.value.read(cx).clone();
        if agent.busy || request.trim().is_empty() {
            return;
        }
        agent.cancel();
        agent.busy = true;
        agent.error = None;
        agent.database = database.clone();
        let generation = agent.generation;
        let task = self.runtime.spawn(async move {
            let work = async {
                let schema = if kind.is_sql() {
                    serde_json::to_value(engine.relational_schema().await.map_err(|_| "Could not load schema metadata for this database".to_string())?).map_err(|e| e.to_string())?
                } else { metadata };
                let context = serde_json::json!({ "database": database, "kind": kind, "dialect": kind.dialect(), "native_query_example": kind.default_query(), "schema": schema });
                crate::agents::generate(cli, config, crate::agents::build_prompt(&context, &request)?).await
            };
            tokio::time::timeout(Duration::from_secs(200), work).await.map_err(|_| "Schema loading or generation timed out.".to_string())?
        });
        if let Some(agent) = self.agent_query_mut(session_id, tab_id) {
            agent.abort.replace(task.abort_handle());
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(agent) = this.agent_query_mut(session_id, tab_id) else {
                    return;
                };
                if agent.generation != generation {
                    return;
                }
                agent.busy = false;
                agent.abort.clear();
                match result {
                    Ok(Ok(result)) => {
                        agent.result = Some(result);
                        agent.generated_by = Some(generated_by);
                    }
                    Ok(Err(error)) => agent.error = Some(error),
                    Err(_) => agent.error = Some("Generation was interrupted.".into()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn choosing_an_agent_makes_it_the_default_and_edits_save_per_cli(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        let saved = store.clone();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.settings_store = Some(store);
            app.select_agent(AgentCli::OpenCode, cx);
            app.agent_setup.executable.set("/opt/opencode".into(), cx);
            app.agent_setup.model.set("test-model".into(), cx);
            app.agent_setup.provider.set("test-provider".into(), cx);
            app.commit_agent_fields(cx);
            app.open_agent_settings(cx);
            app
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("agents-settings").is_some());
        let advanced = cx.debug_bounds("agent-advanced-settings").unwrap();
        cx.simulate_click(advanced.center(), gpui::Modifiers::default());
        assert!(app.read_with(cx, |app, _| app.agent_setup.advanced_open));
        cx.simulate_resize(gpui::size(px(720.), px(640.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("agents-settings").is_some());
        let appearance = cx.debug_bounds("settings-section-Appearance").unwrap();
        cx.simulate_click(appearance.center(), gpui::Modifiers::default());
        assert!(app.read_with(cx, |app, _| app.settings_section
            == SettingsSection::Appearance));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("agents-settings").is_none());
        let assistant = cx.debug_bounds("settings-section-Query assistant").unwrap();
        cx.simulate_click(assistant.center(), gpui::Modifiers::default());
        let preferences = saved.load().unwrap().agents;
        assert_eq!(preferences.default_cli, AgentCli::OpenCode);
        let config = &preferences.configurations[&AgentCli::OpenCode];
        assert_eq!(config.executable, "/opt/opencode");
        assert_eq!(config.model, "test-model");
        assert_eq!(config.provider, "test-provider");

        // Switching shows the other CLI's own configuration and leaves OpenCode's intact.
        app.update(cx, |app, cx| {
            app.agent_setup.model.set("edited-before-switch".into(), cx);
            app.select_agent(AgentCli::Claude, cx);
            assert_eq!(app.agent_setup.executable.text(cx), "claude");
            assert_eq!(app.agent_setup.model.text(cx), "");
            app.select_agent(AgentCli::OpenCode, cx);
            assert_eq!(app.agent_setup.model.text(cx), "edited-before-switch");
            app.close_settings(cx);
        });
        assert_eq!(saved.load().unwrap().agents.default_cli, AgentCli::OpenCode);
    }

    #[gpui::test]
    fn query_preview_does_not_change_editor_and_database_invalidation_clears_it(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Agent test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            let mut query = QueryTab::new(DatabaseKind::SQLite, session_id, tab_id, window, cx);
            query.agent.open = true;
            query.agent.result = Some(GeneratedQuery {
                query: "SELECT count(*) FROM users".into(),
                explanation: "Count users".into(),
            });
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(query)),
            });
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            app.activate_secondary_tab_for(session_id, tab_id, window, cx);
            app
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("generated-query-preview").is_some());
        cx.simulate_resize(gpui::size(px(780.), px(640.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("generated-query-preview").is_some());
        let preview = cx.debug_bounds("generated-query-preview").unwrap();
        let insert = cx.debug_bounds("use-generated-query").unwrap();
        assert!(insert.origin.y >= preview.bottom());
        assert!(insert.right() <= px(780.));
        app.update(cx, |app, cx| {
            let session = app.session_mut(session_id).unwrap();
            let SecondaryTabKind::Query(query) = &mut session.secondary_tabs[0].kind else {
                panic!("query tab");
            };
            assert_ne!(
                query.query_editor.read(cx).text(cx),
                "SELECT count(*) FROM users"
            );
            let generation = query.agent.generation;
            query.invalidate_request();
            assert!(query.agent.generation != generation);
            assert!(query.agent.result.is_none());
            assert!(!query.agent.busy);
        });
    }

    #[gpui::test]
    fn escape_closes_the_prompt_and_use_query_replaces_the_editor(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| {
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                gpui::KeyBinding::new("escape", DismissQueryAgent, Some("DbxQueryAgent")),
                gpui::KeyBinding::new("enter", SubmitQueryAgent, Some("DbxQueryAgent")),
                gpui::KeyBinding::new("shift-enter", editor::Enter, Some("DbxQueryAgent")),
            ]);
        });
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Agent test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(QueryTab::new(
                    DatabaseKind::SQLite,
                    session_id,
                    tab_id,
                    window,
                    cx,
                ))),
            });
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            app.activate_secondary_tab_for(session_id, tab_id, window, cx);
            app.toggle_agent_panel(session_id, tab_id, window, cx);
            app
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("query-agent-panel").is_some());
        cx.simulate_keystrokes("shift-enter");
        assert_eq!(
            app.read_with(cx, |app, cx| app
                .agent_query(session_id, tab_id)
                .unwrap()
                .prompt
                .value
                .read(cx)
                .clone()),
            "\n"
        );
        cx.simulate_input("Count active projects");
        assert!(app.read_with(cx, |app, cx| {
            app.agent_query(session_id, tab_id)
                .unwrap()
                .prompt
                .value
                .read(cx)
                .contains("Count active projects")
        }));
        cx.simulate_keystrokes("escape");
        assert!(!app.read_with(cx, |app, _| app.agent_panel_open(session_id, tab_id)));

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.toggle_agent_panel(session_id, tab_id, window, cx);
                app.agent_query_mut(session_id, tab_id).unwrap().result = Some(GeneratedQuery {
                    query: "SELECT count(*) FROM users".into(),
                    explanation: "Count users".into(),
                });
                app.agent_query_mut(session_id, tab_id).unwrap().error =
                    Some("Previous attempt failed".into());
                app.use_generated_query(session_id, tab_id, false, window, cx);
            });
        });
        app.read_with(cx, |app, cx| {
            let session = app.session(session_id).unwrap();
            let SecondaryTabKind::Query(query) = &session.secondary_tabs[0].kind else {
                panic!("query tab");
            };
            assert_eq!(
                query.query_editor.read(cx).text(cx),
                "SELECT count(*) FROM users"
            );
            assert!(!query.agent.open);
            assert!(query.agent.result.is_none());
            assert!(query.agent.error.is_none());
        });
    }
}
