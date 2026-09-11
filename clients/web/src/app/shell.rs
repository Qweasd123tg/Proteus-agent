use super::{
    commands::ChatCommands, connection::ClientConnection, menus::close_menus_on_outside_click,
    navigation::AppRouter, state::AppState,
};
use crate::components::*;
use leptos::prelude::*;
#[component]
pub(super) fn AppShell(
    state: AppState,
    connection: ClientConnection,
    commands: ChatCommands,
    router: AppRouter,
) -> impl IntoView {
    let super::state::ChatState {
        is_sending,
        active_run_id,
        plan_run_id,
        agent_status,
        tool_activities,
        pending_approvals,
        pending_user_inputs,
        transcript_generation,
        messages,
        ..
    } = state.chat;
    let super::state::RequestState {
        draft,
        set_draft,
        queued_prompts,
        mode,
        model_name,
        model_options,
        reasoning_enabled,
        effort,
        effort_options,
        ..
    } = state.request;
    let super::state::SessionState {
        transport_status,
        event_count,
        workspace_label,
        active_session_dir,
        sidebar_sessions,
        sidebar_sessions_status,
        ..
    } = state.session;
    let super::state::ViewState {
        toasts,
        stick_to_bottom,
        set_stick_to_bottom,
        last_results_scroll_top,
        set_last_results_scroll_top,
        tool_cards_collapsed,
        set_tool_cards_collapsed,
        activity_now_ms,
        detach_baseline,
        results_ref,
        composer_ref,
        resize,
        ..
    } = state.view;
    let user_messages = state.user_messages;
    let route = router.route;
    let actions = connection.actions;
    let session_actions = connection.session_actions;
    let info_panel_open = resize.info_open;
    let topnav_click = move |event, path| router.click(event, path);
    let reconnect_transport = move |_| connection.reconnect();
    let resume_open = move |session| {
        session_actions.open_sidebar_session(session);
        router.navigate("/");
    };
    let start_new_session = move |_| session_actions.start_new_session();
    let refresh_sidebar_sessions = move |_| session_actions.load_sidebar_sessions();
    let open_sidebar_session = move |session| session_actions.open_sidebar_session(session);
    let delete_sidebar_session = move |session| session_actions.delete_sidebar_session(session);
    let toggle_sidebar = move |_| resize.toggle_sidebar();
    let toggle_info_panel = move |_| resize.toggle_info_panel();
    let begin_sidebar_resize = move |event| resize.begin_sidebar_resize(event);
    let begin_info_resize = move |event| resize.begin_info_resize(event);
    let begin_chat_resize = move |event| resize.begin_chat_resize(event);
    let resize_drag = move |event| resize.drag(event);
    let stop_resize = move |_| resize.stop();
    let is_resizing = move || resize.is_resizing();
    let draft_is_empty = move || draft.get().trim().is_empty();
    let new_below_count = move || {
        detach_baseline
            .get()
            .map(|baseline| messages.len().saturating_sub(baseline))
            .unwrap_or(0)
    };
    let waiting_background_sessions = Memo::new(move |_| {
        let active = active_session_dir.get();
        sidebar_sessions.with(|sessions| {
            sessions
                .iter()
                .filter(|session| {
                    Some(session.session_dir.as_str()) != active.as_deref()
                        && session.activity.as_ref().is_some_and(|activity| {
                            activity.pending_approvals > 0 || activity.pending_user_inputs > 0
                        })
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    super::extension_state::publish(state);

    view! {
        <div
            class="app-layout"
            class:chat-route=move || router.is_chat()
            class:resizing=is_resizing
            class:sidebar-collapsed=resize.sidebar_collapsed
            on:mousemove=resize_drag
            on:mouseup=stop_resize
            on:mouseleave=stop_resize
            on:click=close_menus_on_outside_click
        >
            <ToastStack toasts on_dismiss=move |value| commands.dismiss_toast.run(value) />
            <SidebarView
                sidebar_width=resize.sidebar_width
                sidebar_collapsed=resize.sidebar_collapsed
                workspace_label
                sidebar_sessions
                sidebar_sessions_status
                active_session_dir
                on_refresh=refresh_sidebar_sessions
                on_new_session=start_new_session
                on_toggle=toggle_sidebar
                on_begin_resize=begin_sidebar_resize
                on_open_session=open_sidebar_session
                on_delete_session=delete_sidebar_session
            >
                <SidebarFooter route transport_status active_session_dir active_run_id event_count tool_activities
                    on_navigate=topnav_click on_reconnect=reconnect_transport
                    on_cancel=move |value| commands.cancel_turn.run(value) />
            </SidebarView>

            <main class="workspace-main">
                <crate::components::header::HeaderView
                    route workspace_label waiting_background_sessions info_panel_open
                    on_navigate=topnav_click
                    on_open_session=move |session| session_actions.open_sidebar_session(session)
                    on_toggle_info=toggle_info_panel
                />

                <div class="extension-host extension-dock-main" data-extension-location="main"></div>
                <section
                    class="session-workspace"
                    style=move || format!("--chat-max-width: {}px", resize.chat_width.get())
                >
                    {move || {
                        let current = route.get();
                        if current == "/resume" {
                            view! { <ResumeView on_open=resume_open /> }.into_any()
                        } else if current == "/context" {
                        view! {
                            <SessionAnalysisView
                                sessions=sidebar_sessions
                                active_session_dir=active_session_dir
                                on_open=resume_open
                            />
                        }.into_any()
                    } else if current == "/settings" {
                        view! { <SettingsView active_session_dir transcript_generation tool_cards_collapsed set_tool_cards_collapsed /> }.into_any()
                    } else {
                        view! {
                            <ChatResultsView
                                results_ref
                                stick_to_bottom
                                set_stick_to_bottom
                                last_results_scroll_top
                                set_last_results_scroll_top
                                messages
                                activity_now_ms
                                pending_approvals
                                pending_user_inputs
                                queued_prompts
                                plan_run_id
                                is_sending
                                agent_status
                                on_resolve_approval=move |id, approved, cache| commands.resolve_approval.run((id, approved, cache))
                                on_submit_user_input=move |id, answers| commands.submit_user_input.run((id, answers))
                                on_revise_plan=move |value| commands.revise_plan.run(value)
                                on_execute_plan=move |value| commands.execute_plan.run(value)
                                on_exit_plan=move |value| commands.exit_plan.run(value)
                            />

                            <ComposerView
                                queued_prompts
                                composer_ref
                                draft
                                set_draft
                                mode
                                model_name
                                model_options
                                reasoning_enabled
                                effort
                                effort_options
                                is_sending
                                active_run_id
                                stick_to_bottom
                                set_stick_to_bottom
                                actions
                                draft_is_empty
                                new_below_count
                                on_submit=move |value| commands.submit.run(value)
                                on_keydown=move |value| commands.submit_shortcut.run(value)
                                on_cancel_turn=move |value| commands.cancel_turn.run(value)
                            />

                            <div
                                class="chat-resize-handle"
                                aria-hidden="true"
                                title="Ширина чата"
                                on:mousedown=begin_chat_resize
                            ></div>

                            <MessageNav
                                items=user_messages
                                on_jump=move |value| commands.jump_to_message.run(value)
                            />
                        }.into_any()
                    }}}
                </section>
            </main>

            <InfoPanelView open=info_panel_open width=resize.info_width
                on_toggle=toggle_info_panel on_begin_resize=begin_info_resize />
            <crate::components::extensions::ExtensionsView active_session_dir />

        </div>
    }
}
