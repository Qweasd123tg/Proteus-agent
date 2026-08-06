pub(crate) fn extract_search_queries(task: &str) -> Vec<String> {
    let mut queries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in task.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')) {
        let token = raw.trim_matches(|ch: char| ch == '_' || ch == '-');
        if token.len() < 3 || token.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let normalized = token.to_ascii_lowercase();
        let looks_code_like = token.contains('_')
            || token.contains('-')
            || token.chars().any(|ch| ch.is_ascii_uppercase())
            || token.ends_with(".rs")
            || token.ends_with(".toml")
            || token.ends_with(".md")
            || token.ends_with(".json");
        let looks_domain_relevant = REPO_SEARCH_ALLOWLIST.contains(&normalized.as_str())
            || (token.len() >= 4
                && token.chars().all(|ch| ch.is_ascii_lowercase())
                && !REPO_SEARCH_STOPWORDS.contains(&normalized.as_str()));
        if (looks_code_like || looks_domain_relevant) && seen.insert(normalized.clone()) {
            queries.push(token.to_owned());
        }
        if queries.len() >= 4 {
            return queries;
        }
    }
    if queries.is_empty() {
        let fallback = task.trim();
        if !fallback.is_empty() {
            queries.push(fallback.chars().take(80).collect());
        }
    }
    queries
}

const REPO_SEARCH_ALLOWLIST: &[&str] = &[
    "agent",
    "approval",
    "cancel",
    "config",
    "context",
    "event",
    "history",
    "memory",
    "model",
    "module",
    "patch",
    "plugin",
    "policy",
    "provider",
    "renderer",
    "runtime",
    "search",
    "session",
    "shell",
    "stdio",
    "tool",
    "tools",
    "transport",
    "workflow",
];

const REPO_SEARCH_STOPWORDS: &[&str] = &[
    "about", "after", "also", "before", "between", "could", "does", "done", "from", "have", "into",
    "just", "like", "more", "need", "only", "over", "should", "some", "that", "then", "there",
    "this", "what", "when", "where", "while", "with", "without", "would",
];
