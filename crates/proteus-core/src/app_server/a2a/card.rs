use a2a::{AgentCapabilities, AgentCard, AgentExtension, AgentInterface, AgentSkill};

use super::interaction::INTERACTION_EXTENSION;

pub(super) fn agent_card(url: String) -> AgentCard {
    AgentCard {
        name: "Proteus".into(),
        description: "Configured local Proteus agent; each context owns a separate session.".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        supported_interfaces: vec![AgentInterface::new(url, a2a::TRANSPORT_PROTOCOL_JSONRPC)],
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(false),
            extended_agent_card: Some(false),
            extensions: Some(vec![AgentExtension {
                uri: INTERACTION_EXTENSION.into(),
                description: Some(
                    "Proteus approval and typed user input in input-required tasks.".into(),
                ),
                required: Some(false),
                params: None,
            }]),
        },
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![AgentSkill {
            id: "configured-agent".into(),
            name: "Configured agent".into(),
            description: "Runs tasks using the selected Proteus workflow, modules and policy."
                .into(),
            tags: vec!["proteus".into()],
            examples: None,
            input_modes: None,
            output_modes: None,
            security_requirements: None,
        }],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    }
}
