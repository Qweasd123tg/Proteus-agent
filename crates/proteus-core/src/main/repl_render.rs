use anyhow::Result;
use proteus_contracts::domain::AgentOutput;
use serde_json::Value;

pub(super) fn repl_header(config: &Value) -> Result<String> {
    let profile = config_string(config, &["profile"])?;
    let model = config_model_label(config)?.unwrap_or("not selected");
    let cwd = config_string(config, &["cwd"])?;
    let modules = config
        .get("modules")
        .and_then(Value::as_array)
        .map(|modules| {
            modules
                .iter()
                .filter_map(|module| {
                    Some(format!(
                        "{}={}",
                        module.get("slot")?.as_str()?,
                        module.get("id")?.as_str()?
                    ))
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let tools = config
        .get("tools_enabled")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut lines = vec![
        "Proteus REPL".to_owned(),
        "type a task, /help, or /exit".to_owned(),
        format!("profile: {profile}"),
        format!("model: {model}"),
        format!("cwd: {cwd}"),
        format!("modules: {modules}"),
        format!("tools: {tools}"),
    ];
    if let Some(session_dir) = config.get("session_dir").and_then(Value::as_str) {
        lines.push(format!("session: {session_dir}"));
    }
    Ok(small_block("Proteus", &lines))
}

pub(super) fn small_block(title: &str, lines: &[String]) -> String {
    let text_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or_default()
        .max(72);
    let inner_width = text_width + 2;
    let title = format!(" {title} ");
    let right = inner_width.saturating_sub(title.chars().count());
    let mut rendered = format!("╭{}{}╮\n", title, "─".repeat(right));
    for line in lines {
        rendered.push_str(&format!(
            "│ {}{} │\n",
            line,
            " ".repeat(text_width.saturating_sub(line.chars().count()))
        ));
    }
    rendered.push_str(&format!("╰{}╯", "─".repeat(inner_width)));
    rendered
}

pub(super) fn initial_footer(config: &Value) -> Result<String> {
    let model = config_model_label(config)?.unwrap_or("not selected");
    Ok(format!(
        "? for shortcuts    model {model} · Context waiting"
    ))
}

pub(super) fn footer_from_output(config: &Value, output: &AgentOutput) -> Result<String> {
    let model = footer_model(config, output)?;
    let context = footer_context(output);
    let session = output
        .metadata
        .get("session_id")
        .and_then(Value::as_str)
        .map(short_id)
        .unwrap_or("unknown");
    Ok(format!(
        "? for shortcuts    {model} · {context} · session {session}"
    ))
}

fn footer_model(config: &Value, output: &AgentOutput) -> Result<String> {
    if let Some(model) = output.metadata.get("model") {
        let provider = model.get("provider").and_then(Value::as_str);
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str));
        if let Some(name) = name {
            return Ok(match provider {
                Some(provider) if !provider.is_empty() => format!("model {provider}/{name}"),
                _ => format!("model {name}"),
            });
        }
    }

    Ok(format!(
        "model {}",
        config_model_label(config)?.unwrap_or("not selected")
    ))
}

fn config_model_label(config: &Value) -> Result<Option<&str>> {
    let model = config
        .get("model")
        .ok_or_else(|| anyhow::anyhow!("app-server config is missing model"))?;
    if model.is_null() {
        Ok(None)
    } else {
        config_string(config, &["model", "label"]).map(Some)
    }
}

fn config_string<'a>(config: &'a Value, path: &[&str]) -> Result<&'a str> {
    let mut value = config;
    for segment in path {
        value = value
            .get(*segment)
            .ok_or_else(|| anyhow::anyhow!("app-server config is missing {segment}"))?;
    }
    value.as_str().ok_or_else(|| {
        anyhow::anyhow!("app-server config field {} is not a string", path.join("."))
    })
}

fn footer_context(output: &AgentOutput) -> String {
    let context = output.metadata.get("context");
    let tokens = context
        .and_then(|context| context.get("token_estimate"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let chunks = context
        .and_then(|context| context.get("chunks"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let max_tokens = 200_000_u64;
    let percent = ((tokens as f64 / max_tokens as f64) * 100.0).clamp(0.0, 100.0);
    let chunk_word = if chunks == 1 { "chunk" } else { "chunks" };
    format!(
        "Context {:.0}% · {} in · {} {}",
        percent, tokens, chunks, chunk_word
    )
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_free_config_opens_header_and_both_footer_paths() {
        let config = json!({"profile":"check", "model":null, "cwd":"/workspace"});
        assert!(
            repl_header(&config)
                .unwrap()
                .contains("model: not selected")
        );
        assert!(
            initial_footer(&config)
                .unwrap()
                .contains("model not selected")
        );
        assert!(
            footer_from_output(&config, &AgentOutput::text("checked"))
                .unwrap()
                .contains("model not selected")
        );
    }

    #[test]
    fn present_model_is_kept_and_malformed_model_remains_an_error() {
        let config = json!({"profile":"chat", "model":{"label":"fake/test"}, "cwd":"/workspace"});
        assert!(repl_header(&config).unwrap().contains("model: fake/test"));
        assert!(initial_footer(&config).unwrap().contains("model fake/test"));
        assert_eq!(
            footer_model(&config, &AgentOutput::text("done")).unwrap(),
            "model fake/test"
        );
        for model in [json!({}), json!(false), json!("fake/test")] {
            assert!(config_model_label(&json!({"model":model})).is_err());
        }
    }
}
