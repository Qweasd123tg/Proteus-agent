//! Docs-on-disk skills exposed through an external context provider and tool.

mod discovery;

use std::path::Path;

use discovery::{SkillDocument, discover_skills};
use proteus_contracts::{
    domain::ContextChunk,
    process_module::{
        ContextProviderModule, ContextProviderModuleInput, ContextProviderModuleObject,
        ModuleRegistry, ProcessModuleError, ToolModule, ToolModuleHostMut,
        ToolModuleInvocationContext, ToolModuleObject,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

const PROVIDER_ID: &str = "skills";
const TOOL_NAME: &str = "skill";

pub struct SkillsContextProvider;
pub struct SkillTool;

impl ContextProviderModule for SkillsContextProvider {
    fn provide_json(&self, input_json: String) -> Result<String, ProcessModuleError> {
        let input: ContextProviderModuleInput = match serde_json::from_str(input_json.as_str()) {
            Ok(input) => input,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "failed to parse ContextProviderModuleInput: {error}"
                )));
            }
        };
        if input.provider_id != PROVIDER_ID {
            return Err(ProcessModuleError::new(format!(
                "skill-pack received unexpected provider id '{}'",
                input.provider_id
            )));
        }
        let skills = match discover_skills(&input.task.cwd) {
            Ok(skills) => skills,
            Err(error) => return Err(ProcessModuleError::new(error)),
        };
        let chunks = vec![available_skills_chunk(&skills)];
        match serde_json::to_string(&chunks) {
            Ok(output) => Ok(String::from(output)),
            Err(error) => Err(ProcessModuleError::new(format!(
                "failed to serialize skills context: {error}"
            ))),
        }
    }
}

impl ToolModule for SkillTool {
    fn spec_json(&self) -> String {
        String::from(
            json!({
                "name": TOOL_NAME,
                "description": "Load the instruction body of one available docs-on-disk skill by name.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact skill name from <available_skills>."
                        }
                    },
                    "required": ["name"],
                    "additionalProperties": false
                },
                "surface": { "kind": "function", "strict": false, "output_schema": null },
                "safety": "ReadOnly",
                "timeout_ms": 10_000,
                "metadata": {
                    "hot": true,
                    "category": "skills",
                    "tags": ["skill", "instructions", "workflow"],
                    "aliases": ["load skill", "skill instructions", "SKILL.md"]
                }
            })
            .to_string(),
        )
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        _host: &mut ToolModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        let call: ToolCallDto = match serde_json::from_str(call_json.as_str()) {
            Ok(call) => call,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "failed to parse ToolCall: {error}"
                )));
            }
        };
        let context: ToolModuleInvocationContext = match serde_json::from_str(context_json.as_str())
        {
            Ok(context) => context,
            Err(error) => {
                return Err(ProcessModuleError::new(format!(
                    "failed to parse ToolModuleInvocationContext: {error}"
                )));
            }
        };
        let result = invoke_skill(&call, &context.cwd);
        Ok(String::from(result.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallDto {
    id: String,
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillArgs {
    name: String,
}

fn invoke_skill(call: &ToolCallDto, cwd: &Path) -> Value {
    if call.name != TOOL_NAME {
        return tool_error(&call.id, format!("unexpected tool name '{}'", call.name));
    }
    let args: SkillArgs = match serde_json::from_value(call.args.clone()) {
        Ok(args) => args,
        Err(error) => return tool_error(&call.id, format!("invalid skill arguments: {error}")),
    };
    let skills = match discover_skills(cwd) {
        Ok(skills) => skills,
        Err(error) => return tool_error(&call.id, error),
    };
    let Some(skill) = skills.into_iter().find(|skill| skill.name == args.name) else {
        return tool_error(
            &call.id,
            format!("skill '{}' is not available in this workspace", args.name),
        );
    };
    json!({
        "call_id": call.id,
        "ok": true,
        "output": skill.body,
        "content": [],
        "error": null,
        "metadata": {
            "tool": TOOL_NAME,
            "name": skill.name,
            "description": skill.description,
            "path": skill.path,
            "source": skill.source.as_str()
        }
    })
}

fn tool_error(call_id: &str, error: String) -> Value {
    json!({
        "call_id": call_id,
        "ok": false,
        "output": "",
        "content": [],
        "error": error,
        "metadata": { "tool": TOOL_NAME }
    })
}

fn available_skills_chunk(skills: &[SkillDocument]) -> ContextChunk {
    let mut content = String::from("<available_skills>\n");
    for skill in skills {
        content.push_str("<skill>\n");
        content.push_str("<name>");
        content.push_str(&escape_xml(&skill.name));
        content.push_str("</name>\n<description>");
        content.push_str(&escape_xml(&normalize_whitespace(&skill.description)));
        content.push_str("</description>\n<location>");
        content.push_str(&escape_xml(&skill.path.display().to_string()));
        content.push_str("</location>\n</skill>\n");
    }
    content.push_str("</available_skills>");
    ContextChunk::new("repo_aware:skills", content)
        .with_score(0.95)
        .with_metadata(json!({
            "provider": PROVIDER_ID,
            "reason": "docs-on-disk skills available to the current workspace",
            "skill_count": skills.len()
        }))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let provider: ContextProviderModuleObject = Box::new(SkillsContextProvider);
    if let Err(error) = registry.register_context_provider(String::from(PROVIDER_ID), provider) {
        return Err(error);
    }
    let tool: ToolModuleObject = Box::new(SkillTool);
    registry.register_tool(tool)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use proteus_contracts::domain::ToolSpec;

    use super::*;

    #[test]
    fn skill_tool_emits_strict_canonical_spec() {
        serde_json::from_str::<ToolSpec>(SkillTool.spec_json().as_str())
            .expect("skill spec must match strict ToolSpec");
    }

    #[test]
    fn available_skills_context_escapes_frontmatter_text() {
        let skill = SkillDocument {
            name: "review".to_owned(),
            description: "Check <diff> & tests".to_owned(),
            body: "Body".to_owned(),
            path: "/workspace/.proteus/skills/review/SKILL.md".into(),
            source: discovery::SkillSource::Project,
        };

        let chunk = available_skills_chunk(&[skill]);

        assert!(chunk.content.starts_with("<available_skills>\n"));
        assert!(chunk.content.contains("Check &lt;diff&gt; &amp; tests"));
        assert_eq!(chunk.metadata["skill_count"], 1);
    }

    #[test]
    fn skill_tool_loads_body_without_frontmatter() {
        let workspace = tempfile::tempdir().expect("workspace");
        fs::create_dir(workspace.path().join(".git")).expect("git marker");
        let directory = workspace.path().join(".proteus/skills/review");
        fs::create_dir_all(&directory).expect("skill directory");
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: review\ndescription: Review a patch\n---\n\nInspect the diff.\n",
        )
        .expect("skill");
        let call = ToolCallDto {
            id: "call-skill".to_owned(),
            name: TOOL_NAME.to_owned(),
            args: json!({ "name": "review" }),
        };

        let result = invoke_skill(&call, workspace.path());

        assert_eq!(result["ok"], true);
        assert_eq!(result["output"], "Inspect the diff.");
        assert_eq!(result["metadata"]["source"], "project");
    }

    #[test]
    fn unknown_skill_is_a_failed_tool_result() {
        let workspace = tempfile::tempdir().expect("workspace");
        let call = ToolCallDto {
            id: "call-missing".to_owned(),
            name: TOOL_NAME.to_owned(),
            args: json!({ "name": "missing" }),
        };

        let result = invoke_skill(&call, workspace.path());

        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("not available"));
    }
}
