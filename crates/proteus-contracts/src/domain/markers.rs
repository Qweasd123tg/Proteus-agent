//! Общие строковые маркеры межпаковых contracts.
//!
//! Пары producer/consumer перечислены в `docs/pack-contracts.md`. Константы
//! убирают дрейф написания между modules, которые общаются этими маркерами
//! через ABI/JSON границу без compile-time проверки.

/// `CanonicalMessage::name` сообщений, порождённых context builder-ом, а не
/// пользователем. Producer: workflow packs; consumers: compactor packs,
/// history/token accounting.
pub const CONTEXT_MESSAGE_NAME: &str = "context";

/// Metadata key/value для context chunk, содержимое которого уже оформлено
/// context provider-ом как точный model-visible user message. Adapters не
/// добавляют к такому chunk собственный `Context from ...` envelope.
pub const CONTEXT_RENDER_MODE_KEY: &str = "model_visible_render";
pub const CONTEXT_RENDER_MODE_VERBATIM: &str = "verbatim";

/// Открывающий тег generated-блока с окружением (os/arch/shell/cwd).
/// Producer: context builder (`environment` provider); consumer: compactor
/// packs, сохраняющие блок при компакции.
pub const ENVIRONMENT_CONTEXT_TAG: &str = "<environment_context>";

/// Shell, под которым terminal packs исполняют команды (`sh -lc`).
/// Consumers: shell/exec tools (spawn) и `environment` context provider
/// (сообщает модели `<shell>`).
pub const EXEC_SHELL: &str = "sh";
