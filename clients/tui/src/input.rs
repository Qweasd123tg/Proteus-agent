use std::collections::HashSet;

use agent_contracts::{app_protocol::StdioRequest, contracts::ApprovalCacheScope};
use anyhow::Result;
use crossterm::event::{Event as CTerm, KeyCode, KeyEventKind, KeyModifiers};

use crate::{
    commands::{
        handle_plan_review_action, handle_slash_command, request_cancel, resume_session_dir,
        submit_plan_intake_answers,
    },
    driver::{AgentDriver, DriverConfig},
    state::AppState,
};

pub(crate) async fn handle_term_event(
    state: &mut AppState,
    driver: &mut AgentDriver,
    driver_config: &mut DriverConfig,
    canceled_turn_responses: &mut HashSet<String>,
    cancel_request_responses: &mut HashSet<String>,
    ev: CTerm,
) -> Result<bool> {
    match ev {
        CTerm::Key(key) if is_handled_key_event(key.kind) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c') => {
                        if state.has_context_report() {
                            state.close_context_report();
                        } else if state.has_plan_intake() {
                            state.clear_plan_intake();
                        } else if state.has_plan_review() {
                            state.clear_plan_review();
                        } else if state.input_is_empty() {
                            state.arm_or_confirm_quit();
                        } else {
                            state.clear_input();
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            if state.has_context_report() {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        state.close_context_report();
                        return Ok(true);
                    }
                    KeyCode::Up => {
                        state.scroll_context_report_up(1);
                        return Ok(true);
                    }
                    KeyCode::Down => {
                        state.scroll_context_report_down(1);
                        return Ok(true);
                    }
                    KeyCode::PageUp => {
                        state.scroll_context_report_up(8);
                        return Ok(true);
                    }
                    KeyCode::PageDown => {
                        state.scroll_context_report_down(8);
                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }

            if state.has_resume_picker() {
                match key.code {
                    KeyCode::Enter => {
                        if let Some(session_dir) = state.selected_resume_session() {
                            resume_session_dir(state, driver, driver_config, session_dir).await?;
                            canceled_turn_responses.clear();
                            cancel_request_responses.clear();
                        }
                        return Ok(true);
                    }
                    KeyCode::Esc => {
                        state.close_resume_picker();
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        state.backspace_resume_query();
                        return Ok(true);
                    }
                    KeyCode::Char(ch) => {
                        state.type_resume_query_char(ch);
                        return Ok(true);
                    }
                    KeyCode::Tab => {
                        state.move_resume_selection_down(1);
                        return Ok(true);
                    }
                    KeyCode::BackTab => {
                        state.move_resume_selection_up(1);
                        return Ok(true);
                    }
                    KeyCode::Up => {
                        state.move_resume_selection_up(1);
                        return Ok(true);
                    }
                    KeyCode::Down => {
                        state.move_resume_selection_down(1);
                        return Ok(true);
                    }
                    KeyCode::PageUp => {
                        state.move_resume_selection_up(5);
                        return Ok(true);
                    }
                    KeyCode::PageDown => {
                        state.move_resume_selection_down(5);
                        return Ok(true);
                    }
                    _ => return Ok(false),
                }
            }

            if state.has_pending_approval() {
                match key.code {
                    KeyCode::Char('y')
                    | KeyCode::Char('Y')
                    | KeyCode::Char('1')
                    | KeyCode::Char('н')
                    | KeyCode::Char('Н') => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: true,
                                    note: None,
                                    cache: ApprovalCacheScope::None,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    KeyCode::Char('p')
                    | KeyCode::Char('P')
                    | KeyCode::Char('2')
                    | KeyCode::Char('з')
                    | KeyCode::Char('З') => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: true,
                                    note: None,
                                    cache: ApprovalCacheScope::ToolInCwd,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    KeyCode::Char('n')
                    | KeyCode::Char('N')
                    | KeyCode::Char('3')
                    | KeyCode::Char('т')
                    | KeyCode::Char('Т')
                    | KeyCode::Esc => {
                        if let Some(id) = state.take_pending_approval_id() {
                            driver
                                .send(&StdioRequest::Approval {
                                    id: None,
                                    approval_id: id,
                                    approved: false,
                                    note: Some("denied by user".into()),
                                    cache: ApprovalCacheScope::None,
                                })
                                .await?;
                            return Ok(true);
                        }
                    }
                    _ => {}
                }
                return Ok(false);
            }

            if state.has_plan_intake() {
                match key.code {
                    KeyCode::Enter => {
                        if state.plan_intake_selection_submits_immediately()
                            || state.plan_intake_is_last_question()
                        {
                            submit_plan_intake_answers(state, driver).await?;
                        } else {
                            state.move_plan_intake_question_next();
                        }
                        return Ok(true);
                    }
                    KeyCode::Esc => {
                        state.clear_plan_intake();
                        return Ok(true);
                    }
                    KeyCode::Tab | KeyCode::Right => {
                        state.move_plan_intake_question_next();
                        return Ok(true);
                    }
                    KeyCode::BackTab | KeyCode::Left => {
                        state.move_plan_intake_question_prev();
                        return Ok(true);
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        state.move_plan_intake_option_next();
                        return Ok(true);
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        state.move_plan_intake_option_prev();
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        state.backspace_plan_intake_custom();
                        return Ok(true);
                    }
                    KeyCode::Char(' ') => {
                        state.handle_plan_intake_space();
                        return Ok(true);
                    }
                    KeyCode::Char(ch) => {
                        state.type_plan_intake_custom_char(ch);
                        return Ok(true);
                    }
                    _ => {}
                }
                return Ok(false);
            }

            if state.has_plan_review() {
                match key.code {
                    KeyCode::Enter => {
                        if let Some(action) = state.selected_plan_review_action() {
                            handle_plan_review_action(state, driver, driver_config, action).await?;
                        }
                        return Ok(true);
                    }
                    KeyCode::Esc => {
                        state.clear_plan_review();
                        return Ok(true);
                    }
                    KeyCode::Tab | KeyCode::Down | KeyCode::PageDown => {
                        state.move_plan_review_next();
                        return Ok(true);
                    }
                    KeyCode::BackTab | KeyCode::Up | KeyCode::PageUp => {
                        state.move_plan_review_prev();
                        return Ok(true);
                    }
                    KeyCode::Char('1') => {
                        handle_plan_review_action(
                            state,
                            driver,
                            driver_config,
                            crate::visual::PlanReviewAction::ExecuteAuto,
                        )
                        .await?;
                        return Ok(true);
                    }
                    KeyCode::Char('2') => {
                        handle_plan_review_action(
                            state,
                            driver,
                            driver_config,
                            crate::visual::PlanReviewAction::ExecuteNormal,
                        )
                        .await?;
                        return Ok(true);
                    }
                    KeyCode::Char('3') => {
                        state.begin_plan_revision();
                        return Ok(true);
                    }
                    KeyCode::Char('4') => {
                        state.clear_plan_review();
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        state.clear_plan_review();
                        return Ok(true);
                    }
                    KeyCode::Char(ch) => {
                        state.clear_plan_review();
                        state.type_char(ch);
                        return Ok(true);
                    }
                    _ => {}
                }
                return Ok(false);
            }

            match key.code {
                KeyCode::Enter => {
                    if state.complete_partial_slash_suggestion() {
                        return Ok(true);
                    } else if state.pending_model {
                        if let Some(submission) = state.take_input_for_slash_command() {
                            handle_slash_command(
                                state,
                                driver,
                                driver_config,
                                canceled_turn_responses,
                                cancel_request_responses,
                                &submission.text,
                            )
                            .await?;
                        } else if !state.input_is_empty() {
                            state.reject_send_while_busy();
                        }
                        return Ok(true);
                    } else if let Some(submission) = state.take_input_for_send() {
                        if submission.text.starts_with('/') {
                            handle_slash_command(
                                state,
                                driver,
                                driver_config,
                                canceled_turn_responses,
                                cancel_request_responses,
                                &submission.text,
                            )
                            .await?;
                        } else {
                            let turn_id = state.next_turn_id();
                            let request_text = if state.is_plan_mode() {
                                plan_mode_prompt(&submission.text)
                            } else {
                                submission.text.clone()
                            };
                            driver
                                .send(&StdioRequest::Send {
                                    id: Some(turn_id.clone()),
                                    text: request_text,
                                })
                                .await?;
                            state.mark_user_sent(submission.text, submission.paste_ranges, turn_id);
                        }
                        return Ok(true);
                    }
                }
                KeyCode::Tab => {
                    if state.has_slash_suggestions() {
                        state.complete_slash_suggestion();
                        return Ok(true);
                    }
                }
                KeyCode::BackTab => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                }
                KeyCode::Backspace => {
                    state.backspace();
                    return Ok(true);
                }
                KeyCode::Char(ch) => {
                    state.type_char(ch);
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                }
                KeyCode::PageDown => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                }
                KeyCode::Up => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_prev();
                        return Ok(true);
                    }
                }
                KeyCode::Down => {
                    if state.has_slash_suggestions() {
                        state.move_slash_selection_next();
                        return Ok(true);
                    }
                }
                KeyCode::Right => {
                    if state.complete_slash_suggestion() {
                        return Ok(true);
                    }
                }
                KeyCode::Esc => {
                    if state.pending_model
                        && request_cancel(
                            state,
                            driver,
                            canceled_turn_responses,
                            cancel_request_responses,
                        )
                        .await?
                    {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        CTerm::Paste(text) => {
            state.paste_text(&text);
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

fn is_handled_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn plan_mode_prompt(task: &str) -> String {
    format!(
        "Start a plan-mode requirements interview for this task. Inspect only what is needed with read-only tools. Do not modify files, run write tools, or execute shell/network commands. Do not write the implementation plan yet if any material requirement, preference, stack, scope, output format, or deployment choice is missing. For broad or underspecified tasks, first call AskUserQuestion or request_user_input with one focused multiple-choice question, wait for the user's answer, then ask the next dependent question or produce the final concise staged plan. Ask only questions that materially affect the result. Do not ask whether the plan is approved; the UI handles plan approval after the final plan.\n\nTask:\n{task}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handled_key_events_include_repeat_events() {
        assert!(is_handled_key_event(KeyEventKind::Press));
        assert!(is_handled_key_event(KeyEventKind::Repeat));
        assert!(!is_handled_key_event(KeyEventKind::Release));
    }

    #[test]
    fn plan_mode_prompt_wraps_task_as_read_only_planning_request() {
        let prompt = plan_mode_prompt("fix the TUI");

        assert!(prompt.contains("Start a plan-mode requirements interview"));
        assert!(prompt.contains("Do not modify files"));
        assert!(prompt.contains("Do not write the implementation plan yet"));
        assert!(prompt.contains("AskUserQuestion"));
        assert!(prompt.contains("request_user_input"));
        assert!(prompt.contains("Task:\nfix the TUI"));
    }
}
