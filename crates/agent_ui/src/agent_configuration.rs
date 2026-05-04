pub mod configure_context_server_modal;
mod configure_context_server_tools_modal;
mod manage_profiles_modal;
mod tool_picker;

use std::{rc::Rc, sync::Arc};

use agent::ContextServerRegistry;
use context_server::ContextServerId;
use extension::ExtensionManifest;
use extension_host::ExtensionStore;
use fs::Fs;
use gpui::{
    Action, Anchor, App, Entity, EventEmitter, FocusHandle, Focusable,
    ScrollHandle, Subscription, Task, WeakEntity,
};
use itertools::Itertools;
use language::LanguageRegistry;
use notifications::status_toast::StatusToast;
use project::{
    agent_server_store::{AgentId, AgentServerStore, ExternalAgentSource},
    context_server_store::{ContextServerConfiguration, ContextServerStatus, ContextServerStore},
};
use settings::update_settings_file;
use ui::{
    AiSettingItem, AiSettingItemSource, AiSettingItemStatus, ButtonStyle, ContextMenu,
    Divider, DividerColor, LabelSize, PopoverMenu,
    Switch, Tooltip, WithScrollbar, prelude::*,
};
use util::ResultExt as _;
use workspace::Workspace;
use zed_actions::ExtensionCategoryFilter;

pub(crate) use configure_context_server_modal::ConfigureContextServerModal;
pub(crate) use configure_context_server_tools_modal::ConfigureContextServerToolsModal;
pub(crate) use manage_profiles_modal::ManageProfilesModal;

use crate::{
    Agent,
    agent_connection_store::{AgentConnectionStatus, AgentConnectionStore},
};

pub struct AgentConfiguration {
    fs: Arc<dyn Fs>,
    language_registry: Arc<LanguageRegistry>,
    agent_server_store: Entity<AgentServerStore>,
    agent_connection_store: Entity<AgentConnectionStore>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    context_server_store: Entity<ContextServerStore>,
    context_server_registry: Entity<ContextServerRegistry>,
    _subscriptions: Vec<Subscription>,
    scroll_handle: ScrollHandle,
}

impl AgentConfiguration {
    pub fn new(
        fs: Arc<dyn Fs>,
        agent_server_store: Entity<AgentServerStore>,
        agent_connection_store: Entity<AgentConnectionStore>,
        context_server_store: Entity<ContextServerStore>,
        context_server_registry: Entity<ContextServerRegistry>,
        language_registry: Arc<LanguageRegistry>,
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let subscriptions = vec![
            cx.subscribe(&agent_server_store, |_, _, _, cx| cx.notify()),
            cx.observe(&agent_connection_store, |_, _, cx| cx.notify()),
            cx.subscribe(&context_server_store, |_, _, _, cx| cx.notify()),
        ];

        Self {
            fs,
            language_registry,
            workspace,
            focus_handle,
            agent_server_store,
            agent_connection_store,
            context_server_store,
            context_server_registry,
            _subscriptions: subscriptions,
            scroll_handle: ScrollHandle::new(),
        }
    }
}

impl Focusable for AgentConfiguration {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub enum AssistantConfigurationEvent {}

impl EventEmitter<AssistantConfigurationEvent> for AgentConfiguration {}

enum AgentIcon {
    Name(IconName),
    Path(SharedString),
}

impl AgentConfiguration {
    fn render_section_title(
        &mut self,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        menu: AnyElement,
    ) -> impl IntoElement {
        h_flex()
            .p_4()
            .pb_0()
            .mb_2p5()
            .items_start()
            .justify_between()
            .child(
                v_flex()
                    .w_full()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .pr_1()
                            .w_full()
                            .gap_2()
                            .justify_between()
                            .flex_wrap()
                            .child(Headline::new(title.into()))
                            .child(menu),
                    )
                    .child(Label::new(description.into()).color(Color::Muted)),
            )
    }

    fn render_context_servers_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let context_server_ids = self.context_server_store.read(cx).server_ids();

        let add_server_popover = PopoverMenu::new("add-server-popover")
            .trigger(
                Button::new("add-server", "Add Server")
                    .style(ButtonStyle::Outlined)
                    .start_icon(
                        Icon::new(IconName::Plus)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .label_size(LabelSize::Small),
            )
            .menu({
                move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _window, _cx| {
                        menu.entry("Add Custom Server", None, {
                            |window, cx| {
                                window.dispatch_action(crate::AddContextServer.boxed_clone(), cx)
                            }
                        })
                        .entry("Install from Extensions", None, {
                            |window, cx| {
                                window.dispatch_action(
                                    zed_actions::Extensions {
                                        category_filter: Some(
                                            ExtensionCategoryFilter::ContextServers,
                                        ),
                                        id: None,
                                    }
                                    .boxed_clone(),
                                    cx,
                                )
                            }
                        })
                    }))
                }
            })
            .anchor(gpui::Anchor::TopRight)
            .offset(gpui::Point {
                x: px(0.0),
                y: px(2.0),
            });

        v_flex()
            .min_w_0()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(self.render_section_title(
                "Model Context Protocol (MCP) Servers",
                "All MCP servers connected directly or via a Zed extension.",
                add_server_popover.into_any_element(),
            ))
            .child(
                v_flex()
                    .pl_4()
                    .pb_4()
                    .pr_5()
                    .w_full()
                    .gap_1()
                    .map(|parent| {
                        if context_server_ids.is_empty() {
                            parent.child(
                                h_flex()
                                    .p_4()
                                    .justify_center()
                                    .border_1()
                                    .border_dashed()
                                    .border_color(cx.theme().colors().border.opacity(0.6))
                                    .rounded_sm()
                                    .child(
                                        Label::new("No MCP servers added yet.")
                                            .color(Color::Muted)
                                            .size(LabelSize::Small),
                                    ),
                            )
                        } else {
                            parent.children(itertools::intersperse_with(
                                context_server_ids.iter().cloned().map(|context_server_id| {
                                    self.render_context_server(context_server_id, cx)
                                        .into_any_element()
                                }),
                                || {
                                    Divider::horizontal()
                                        .color(DividerColor::BorderFaded)
                                        .into_any_element()
                                },
                            ))
                        }
                    }),
            )
    }

    fn render_context_server(
        &self,
        context_server_id: ContextServerId,
        cx: &Context<Self>,
    ) -> impl use<> + IntoElement {
        let server_status = self
            .context_server_store
            .read(cx)
            .status_for_server(&context_server_id)
            .unwrap_or(ContextServerStatus::Stopped);
        let server_configuration = self
            .context_server_store
            .read(cx)
            .configuration_for_server(&context_server_id);

        let is_running = matches!(server_status, ContextServerStatus::Running);
        let item_id = SharedString::from(context_server_id.0.clone());
        // Servers without a configuration can only be provided by extensions.
        let provided_by_extension = server_configuration.as_ref().is_none_or(|config| {
            matches!(
                config.as_ref(),
                ContextServerConfiguration::Extension { .. }
            )
        });

        let display_name = if provided_by_extension {
            resolve_extension_for_context_server(&context_server_id, cx)
                .map(|(_, manifest)| {
                    let name = manifest.name.as_str();
                    let stripped = name
                        .strip_suffix(" MCP Server")
                        .or_else(|| name.strip_suffix(" MCP"))
                        .or_else(|| name.strip_suffix(" Context Server"))
                        .unwrap_or(name);
                    SharedString::from(stripped.to_string())
                })
                .unwrap_or_else(|| item_id.clone())
        } else {
            item_id.clone()
        };

        let error = if let ContextServerStatus::Error(error) = server_status.clone() {
            Some(error)
        } else {
            None
        };
        let auth_required = matches!(server_status, ContextServerStatus::AuthRequired);
        let authenticating = matches!(server_status, ContextServerStatus::Authenticating);
        let context_server_store = self.context_server_store.clone();

        let tool_count = self
            .context_server_registry
            .read(cx)
            .tools_for_server(&context_server_id)
            .count();

        let source = if provided_by_extension {
            AiSettingItemSource::Extension
        } else {
            AiSettingItemSource::Custom
        };

        let status = match server_status {
            ContextServerStatus::Starting => AiSettingItemStatus::Starting,
            ContextServerStatus::Running => AiSettingItemStatus::Running,
            ContextServerStatus::Error(_) => AiSettingItemStatus::Error,
            ContextServerStatus::Stopped => AiSettingItemStatus::Stopped,
            ContextServerStatus::AuthRequired => AiSettingItemStatus::AuthRequired,
            ContextServerStatus::Authenticating => AiSettingItemStatus::Authenticating,
        };

        let is_remote = server_configuration
            .as_ref()
            .map(|config| matches!(config.as_ref(), ContextServerConfiguration::Http { .. }))
            .unwrap_or(false);

        let should_show_logout_button = server_configuration.as_ref().is_some_and(|config| {
            matches!(config.as_ref(), ContextServerConfiguration::Http { .. })
                && !config.has_static_auth_header()
        });

        let context_server_configuration_menu = PopoverMenu::new("context-server-config-menu")
            .trigger_with_tooltip(
                IconButton::new("context-server-config-menu", IconName::Settings)
                    .icon_color(Color::Muted)
                    .icon_size(IconSize::Small),
                Tooltip::text("Configure MCP Server"),
            )
            .anchor(Anchor::TopRight)
            .menu({
                let fs = self.fs.clone();
                let context_server_id = context_server_id.clone();
                let language_registry = self.language_registry.clone();
                let workspace = self.workspace.clone();
                let context_server_registry = self.context_server_registry.clone();
                let context_server_store = context_server_store.clone();

                move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _window, _cx| {
                        menu.entry("Configure Server", None, {
                            let context_server_id = context_server_id.clone();
                            let language_registry = language_registry.clone();
                            let workspace = workspace.clone();
                            move |window, cx| {
                                if is_remote {
                                    crate::agent_configuration::configure_context_server_modal::ConfigureContextServerModal::show_modal_for_existing_server(
                                        context_server_id.clone(),
                                        language_registry.clone(),
                                        workspace.clone(),
                                        window,
                                        cx,
                                    )
                                    .detach();
                                } else {
                                    ConfigureContextServerModal::show_modal_for_existing_server(
                                        context_server_id.clone(),
                                        language_registry.clone(),
                                        workspace.clone(),
                                        window,
                                        cx,
                                    )
                                    .detach();
                                }
                            }
                        }).when(tool_count > 0, |this| this.entry("View Tools", None, {
                            let context_server_id = context_server_id.clone();
                            let context_server_registry = context_server_registry.clone();
                            let workspace = workspace.clone();
                            move |window, cx| {
                                let context_server_id = context_server_id.clone();
                                workspace.update(cx, |workspace, cx| {
                                    ConfigureContextServerToolsModal::toggle(
                                        context_server_id,
                                        context_server_registry.clone(),
                                        workspace,
                                        window,
                                        cx,
                                    );
                                })
                                .ok();
                            }
                        }))
                        .when(should_show_logout_button, |this| {
                            this.entry("Log Out", None, {
                                let context_server_store = context_server_store.clone();
                                let context_server_id = context_server_id.clone();
                                move |_window, cx| {
                                    context_server_store.update(cx, |store, cx| {
                                        store.logout_server(&context_server_id, cx).log_err();
                                    });
                                }
                            })
                        })
                        .separator()
                        .entry("Uninstall", None, {
                            let fs = fs.clone();
                            let context_server_id = context_server_id.clone();
                            let workspace = workspace.clone();
                            move |_, cx| {
                                let uninstall_extension_task = match (
                                    provided_by_extension,
                                    resolve_extension_for_context_server(&context_server_id, cx),
                                ) {
                                    (true, Some((id, manifest))) => {
                                        if extension_only_provides_context_server(manifest.as_ref())
                                        {
                                            ExtensionStore::global(cx).update(cx, |store, cx| {
                                                store.uninstall_extension(id, cx)
                                            })
                                        } else {
                                            workspace.update(cx, |workspace, cx| {
                                                show_unable_to_uninstall_extension_with_context_server(workspace, context_server_id.clone(), cx);
                                            }).log_err();
                                            Task::ready(Ok(()))
                                        }
                                    }
                                    _ => Task::ready(Ok(())),
                                };

                                cx.spawn({
                                    let fs = fs.clone();
                                    let context_server_id = context_server_id.clone();
                                    async move |cx| {
                                        uninstall_extension_task.await?;
                                        cx.update(|cx| {
                                            update_settings_file(
                                                fs.clone(),
                                                cx,
                                                {
                                                    let context_server_id =
                                                        context_server_id.clone();
                                                    move |settings, _| {
                                                        settings.project
                                                            .context_servers
                                                            .remove(&context_server_id.0);
                                                    }
                                                },
                                            )
                                        });
                                        anyhow::Ok(())
                                    }
                                })
                                .detach_and_log_err(cx);
                            }
                        })
                    }))
                }
            });

        let feedback_base_container =
            || h_flex().py_1().min_w_0().w_full().gap_1().justify_between();

        let details: Option<AnyElement> = if let Some(error) = error {
            Some(
                feedback_base_container()
                    .child(
                        h_flex()
                            .pr_4()
                            .min_w_0()
                            .w_full()
                            .gap_2()
                            .child(
                                Icon::new(IconName::XCircle)
                                    .size(IconSize::XSmall)
                                    .color(Color::Error),
                            )
                            .child(div().min_w_0().flex_1().child(
                                Label::new(error).color(Color::Muted).size(LabelSize::Small),
                            )),
                    )
                    .when(should_show_logout_button, |this| {
                        this.child(
                            Button::new("error-logout-server", "Log Out")
                                .style(ButtonStyle::Outlined)
                                .label_size(LabelSize::Small)
                                .on_click({
                                    let context_server_store = context_server_store.clone();
                                    let context_server_id = context_server_id.clone();
                                    move |_event, _window, cx| {
                                        context_server_store.update(cx, |store, cx| {
                                            store.logout_server(&context_server_id, cx).log_err();
                                        });
                                    }
                                }),
                        )
                    })
                    .into_any_element(),
            )
        } else if auth_required {
            Some(
                feedback_base_container()
                    .child(
                        h_flex()
                            .pr_4()
                            .min_w_0()
                            .w_full()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Info)
                                    .size(IconSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("Authenticate to connect this server")
                                    .color(Color::Muted)
                                    .size(LabelSize::Small),
                            ),
                    )
                    .child(
                        Button::new("error-logout-server", "Authenticate")
                            .style(ButtonStyle::Outlined)
                            .label_size(LabelSize::Small)
                            .on_click({
                                let context_server_id = context_server_id.clone();
                                move |_event, _window, cx| {
                                    context_server_store.update(cx, |store, cx| {
                                        store.authenticate_server(&context_server_id, cx).log_err();
                                    });
                                }
                            }),
                    )
                    .into_any_element(),
            )
        } else if authenticating {
            Some(
                h_flex()
                    .mt_1()
                    .pr_4()
                    .min_w_0()
                    .w_full()
                    .gap_2()
                    .child(div().size_3().flex_shrink_0())
                    .child(
                        Label::new("Authenticating…")
                            .color(Color::Muted)
                            .size(LabelSize::Small),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };

        let tool_label = if is_running {
            Some(if tool_count == 1 {
                SharedString::from("1 tool")
            } else {
                SharedString::from(format!("{} tools", tool_count))
            })
        } else {
            None
        };

        AiSettingItem::new(item_id, display_name, status, source)
            .action(context_server_configuration_menu)
            .action(
                Switch::new("context-server-switch", is_running.into()).on_click({
                    let context_server_manager = self.context_server_store.clone();
                    let fs = self.fs.clone();

                    move |state, _window, cx| {
                        let is_enabled = match state {
                            ToggleState::Unselected | ToggleState::Indeterminate => {
                                context_server_manager.update(cx, |this, cx| {
                                    this.stop_server(&context_server_id, cx).log_err();
                                });
                                false
                            }
                            ToggleState::Selected => {
                                context_server_manager.update(cx, |this, cx| {
                                    if let Some(server) = this.get_server(&context_server_id) {
                                        this.start_server(server, cx);
                                    }
                                });
                                true
                            }
                        };
                        update_settings_file(fs.clone(), cx, {
                            let context_server_id = context_server_id.clone();

                            move |settings, _| {
                                settings
                                    .project
                                    .context_servers
                                    .entry(context_server_id.0)
                                    .or_insert_with(|| {
                                        settings::ContextServerSettingsContent::Extension {
                                            enabled: is_enabled,
                                            remote: false,
                                            settings: serde_json::json!({}),
                                        }
                                    })
                                    .set_enabled(is_enabled);
                            }
                        });
                    }
                }),
            )
            .when_some(tool_label, |this, label| this.detail_label(label))
            .when_some(details, |this, details| this.details(details))
    }

    fn render_agent_servers_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_server_store = self.agent_server_store.read(cx);

        let agents = agent_server_store
            .external_agents()
            .cloned()
            .collect::<Vec<_>>();

        let agents: Vec<_> = agents
            .into_iter()
            .map(|name| {
                let icon = if let Some(icon_path) = agent_server_store.agent_icon(&name) {
                    AgentIcon::Path(icon_path)
                } else {
                    AgentIcon::Name(IconName::Sparkle)
                };
                let display_name = agent_server_store
                    .agent_display_name(&name)
                    .unwrap_or_else(|| name.0.clone());
                let source = agent_server_store.agent_source(&name).unwrap_or_default();
                (name, icon, display_name, source)
            })
            .sorted_unstable_by_key(|(_, _, display_name, _)| display_name.to_lowercase())
            .collect();

        v_flex()
            .min_w_0()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                v_flex()
                    .child(
                        v_flex()
                            .p_4()
                            .gap_2()
                            .children(Itertools::intersperse_with(
                                agents
                                    .into_iter()
                                    .map(|(name, icon, display_name, source)| {
                                        self.render_agent_server(
                                            icon,
                                            name,
                                            display_name,
                                            source,
                                            cx,
                                        )
                                        .into_any_element()
                                    }),
                                || {
                                    Divider::horizontal()
                                        .color(DividerColor::BorderFaded)
                                        .into_any_element()
                                },
                            )),
                    ),
            )
    }

    fn render_agent_server(
        &self,
        icon: AgentIcon,
        id: impl Into<SharedString>,
        display_name: impl Into<SharedString>,
        source: ExternalAgentSource,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = id.into();
        let display_name = display_name.into();

        let icon = match icon {
            AgentIcon::Name(icon_name) => Icon::new(icon_name)
                .size(IconSize::Small)
                .color(Color::Muted),
            AgentIcon::Path(icon_path) => Icon::from_external_svg(icon_path)
                .size(IconSize::Small)
                .color(Color::Muted),
        };

        let source_kind = match source {
            ExternalAgentSource::Extension => AiSettingItemSource::Extension,
            ExternalAgentSource::Registry => AiSettingItemSource::Registry,
            ExternalAgentSource::Custom => AiSettingItemSource::Custom,
        };

        let agent_server_name = AgentId(id.clone());
        let agent = Agent::Custom {
            id: agent_server_name.clone(),
        };

        let connection_status = self
            .agent_connection_store
            .read(cx)
            .connection_status(&agent, cx);

        let restart_button = matches!(
            connection_status,
            AgentConnectionStatus::Connected | AgentConnectionStatus::Connecting
        )
        .then(|| {
            IconButton::new(
                SharedString::from(format!("restart-{}", id)),
                IconName::RotateCw,
            )
            .disabled(connection_status == AgentConnectionStatus::Connecting)
            .icon_color(Color::Muted)
            .icon_size(IconSize::Small)
            .tooltip(Tooltip::text("Restart Agent Connection"))
            .on_click(cx.listener({
                let agent = agent.clone();
                move |this, _, _window, cx| {
                    let server: Rc<dyn agent_servers::AgentServer> =
                        Rc::new(agent_servers::CustomAgentServer::new(agent.id()));
                    this.agent_connection_store.update(cx, |store, cx| {
                        store.restart_connection(agent.clone(), server, cx);
                    });
                }
            }))
        });

        let uninstall_button = match source {
            ExternalAgentSource::Extension => Some(
                IconButton::new(
                    SharedString::from(format!("uninstall-{}", id)),
                    IconName::Trash,
                )
                .icon_color(Color::Muted)
                .icon_size(IconSize::Small)
                .tooltip(Tooltip::text("Uninstall Agent Extension"))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    let agent_name = agent_server_name.clone();

                    if let Some(ext_id) = this.agent_server_store.update(cx, |store, _cx| {
                        store.get_extension_id_for_agent(&agent_name)
                    }) {
                        ExtensionStore::global(cx)
                            .update(cx, |store, cx| store.uninstall_extension(ext_id, cx))
                            .detach_and_log_err(cx);
                    }
                })),
            ),
            ExternalAgentSource::Registry => {
                let fs = self.fs.clone();
                Some(
                    IconButton::new(
                        SharedString::from(format!("uninstall-{}", id)),
                        IconName::Trash,
                    )
                    .icon_color(Color::Muted)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Remove Registry Agent"))
                    .on_click(cx.listener(move |_, _, _window, cx| {
                        let agent_name = agent_server_name.clone();
                        update_settings_file(fs.clone(), cx, move |settings, _| {
                            let Some(agent_servers) = settings.agent_servers.as_mut() else {
                                return;
                            };
                            if let Some(entry) = agent_servers.get(agent_name.0.as_ref())
                                && matches!(
                                    entry,
                                    settings::CustomAgentServerSettings::Registry { .. }
                                )
                            {
                                agent_servers.remove(agent_name.0.as_ref());
                            }
                        });
                    })),
                )
            }
            ExternalAgentSource::Custom => {
                let fs = self.fs.clone();
                Some(
                    IconButton::new(
                        SharedString::from(format!("uninstall-{}", id)),
                        IconName::Trash,
                    )
                    .icon_color(Color::Muted)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Remove Custom Agent"))
                    .on_click(cx.listener(move |_, _, _window, cx| {
                        let agent_name = agent_server_name.clone();
                        update_settings_file(fs.clone(), cx, move |settings, _| {
                            let Some(agent_servers) = settings.agent_servers.as_mut() else {
                                return;
                            };
                            if let Some(entry) = agent_servers.get(agent_name.0.as_ref())
                                && matches!(
                                    entry,
                                    settings::CustomAgentServerSettings::Custom { .. }
                                )
                            {
                                agent_servers.remove(agent_name.0.as_ref());
                            }
                        });
                    })),
                )
            }
        };

        let status = match connection_status {
            AgentConnectionStatus::Disconnected => AiSettingItemStatus::Stopped,
            AgentConnectionStatus::Connecting => AiSettingItemStatus::Starting,
            AgentConnectionStatus::Connected => AiSettingItemStatus::Running,
        };

        AiSettingItem::new(id, display_name, status, source_kind)
            .icon(icon)
            .when_some(restart_button, |this, button| this.action(button))
            .when_some(uninstall_button, |this, button| this.action(button))
    }
}

impl Render for AgentConfiguration {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("assistant-configuration")
            .key_context("AgentConfiguration")
            .track_focus(&self.focus_handle(cx))
            .relative()
            .size_full()
            .pb_8()
            .bg(cx.theme().colors().panel_background)
            .child(
                div()
                    .size_full()
                    .child(
                        v_flex()
                            .id("assistant-configuration-content")
                            .track_scroll(&self.scroll_handle)
                            .size_full()
                            .min_w_0()
                            .overflow_y_scroll()
                            .child(self.render_agent_servers_section(cx))
                            .child(self.render_context_servers_section(cx)),
                    )
                    .vertical_scrollbar_for(&self.scroll_handle, window, cx),
            )
    }
}

fn extension_only_provides_context_server(manifest: &ExtensionManifest) -> bool {
    manifest.context_servers.len() == 1
        && manifest.themes.is_empty()
        && manifest.icon_themes.is_empty()
        && manifest.languages.is_empty()
        && manifest.grammars.is_empty()
        && manifest.language_servers.is_empty()
        && manifest.slash_commands.is_empty()
        && manifest.snippets.is_none()
        && manifest.debug_locators.is_empty()
}

pub(crate) fn resolve_extension_for_context_server(
    id: &ContextServerId,
    cx: &App,
) -> Option<(Arc<str>, Arc<ExtensionManifest>)> {
    ExtensionStore::global(cx)
        .read(cx)
        .installed_extensions()
        .iter()
        .find(|(_, entry)| entry.manifest.context_servers.contains_key(&id.0))
        .map(|(id, entry)| (id.clone(), entry.manifest.clone()))
}

// This notification appears when trying to delete
// an MCP server extension that not only provides
// the server, but other things, too, like language servers and more.
fn show_unable_to_uninstall_extension_with_context_server(
    workspace: &mut Workspace,
    id: ContextServerId,
    cx: &mut App,
) {
    let workspace_handle = workspace.weak_handle();
    let context_server_id = id.clone();

    let status_toast = StatusToast::new(
        format!(
            "The {} extension provides more than just the MCP server. Proceed to uninstall anyway?",
            id.0
        ),
        cx,
        move |this, _cx| {
            let workspace_handle = workspace_handle.clone();

            this.icon(
                Icon::new(IconName::Warning)
                    .size(IconSize::Small)
                    .color(Color::Warning),
            )
            .dismiss_button(true)
            .action("Uninstall", move |_, _cx| {
                if let Some((extension_id, _)) =
                    resolve_extension_for_context_server(&context_server_id, _cx)
                {
                    ExtensionStore::global(_cx).update(_cx, |store, cx| {
                        store
                            .uninstall_extension(extension_id, cx)
                            .detach_and_log_err(cx);
                    });

                    workspace_handle
                        .update(_cx, |workspace, cx| {
                            let fs = workspace.app_state().fs.clone();
                            cx.spawn({
                                let context_server_id = context_server_id.clone();
                                async move |_workspace_handle, cx| {
                                    cx.update(|cx| {
                                        update_settings_file(fs, cx, move |settings, _| {
                                            settings
                                                .project
                                                .context_servers
                                                .remove(&context_server_id.0);
                                        });
                                    });
                                    anyhow::Ok(())
                                }
                            })
                            .detach_and_log_err(cx);
                        })
                        .log_err();
                }
            })
        },
    );

    workspace.toggle_status_toast(status_toast, cx);
}

