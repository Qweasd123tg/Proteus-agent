export const files = {
  'Cargo.toml': '[package]\nname = "atlas"\nversion = "0.4.0"\nedition = "2024"\n\n[dependencies]\nserde = { version = "1", features = ["derive"] }\ntoml = "0.8"\n',
  'README.md': '# Atlas\n\nНебольшая CLI-утилита для проверки конфигураций.\n\n```sh\natlas check config.toml --strict\n```\n',
  'docs/guide.md': '# Руководство\n\nФлаг `--strict` превращает предупреждения в ошибки.\n',
  'src/main.rs': 'mod config;\nmod parser;\n\nfn main() {\n    let cfg = config::load("config.toml").expect("config");\n    println!("strict = {}", cfg.strict);\n}\n',
  'src/config.rs': 'use serde::Deserialize;\n\n#[derive(Deserialize)]\npub struct Config {\n    pub name: String,\n    #[serde(default)]\n    pub strict: bool,\n}\n\npub fn load(path: &str) -> anyhow::Result<Config> {\n    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)\n}\n',
  'src/parser/mod.rs': 'pub mod tokens;\n\npub use tokens::Token;\n',
  'src/parser/tokens.rs': '#[derive(Debug, PartialEq)]\npub enum Token {\n    Key(String),\n    Value(String),\n}\n',
  'tests/config.rs': '#[test]\nfn strict_defaults_to_false() {\n    let cfg: atlas::Config = toml::from_str("name = \\"x\\"").unwrap();\n    assert!(!cfg.strict);\n}\n',
};
export const changes = { 'src/config.rs': 'modified', 'README.md': 'modified', 'tests/config.rs': 'added' };
export const patches = {
  'src/config.rs': 'diff --git a/src/config.rs b/src/config.rs\n--- a/src/config.rs\n+++ b/src/config.rs\n@@ -3,5 +3,7 @@ use serde::Deserialize;\n #[derive(Deserialize)]\n pub struct Config {\n     pub name: String,\n+    #[serde(default)]\n+    pub strict: bool,\n }\n',
  'README.md': 'diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -3,3 +3,7 @@\n Небольшая CLI-утилита для проверки конфигураций.\n+\n+```sh\n+atlas check config.toml --strict\n+```\n',
  'tests/config.rs': 'diff --git a/tests/config.rs b/tests/config.rs\nnew file mode 100644\n--- /dev/null\n+++ b/tests/config.rs\n@@ -0,0 +1,5 @@\n+#[test]\n+fn strict_defaults_to_false() {\n+    let cfg: atlas::Config = toml::from_str("name = \\"x\\"").unwrap();\n+    assert!(!cfg.strict);\n+}\n',
};
