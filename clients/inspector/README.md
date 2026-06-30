# Inspector Web Client

Отдельный Leptos/Trunk клиент для редко используемых config/architecture
экранов. Он подключается к тому же локальному `proteus server http`, но не
поднимает чатовый transcript, SSE event stream, composer, approvals или
runtime-control state.

Текущий состав:

- `/architecture` читает `/inspect/topology` и `/inspect/topology.mmd`,
  показывает topology map, runtime pipeline, slots, tools, plugin
  contributions и warnings;
- `/configs` читает `/config` и `/config/builder`, показывает active modules,
  tools, plugins, model/reasoning и config files, а также даёт Config builder
  для выбора реализации каждого `[modules]` slot-а и редактирования
  `module_config.<slot>.<module_id>` JSON payload.

## Запуск

Требуется wasm target и Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo run --bin proteus -- server http --port 8787
```

В другом терминале:

```bash
cd clients/inspector
env -u NO_COLOR trunk serve
```

По умолчанию inspector слушает `http://127.0.0.1:1421`, chat-клиент —
`http://127.0.0.1:1420`, app-server — `http://127.0.0.1:8787`.
Default CORS app-server разрешает оба web-порта.

Обычный wrapper после `./install.sh` поднимает Inspector вместе с chat-клиентом.
Чтобы оставить только chat loop, запускайте `PROTEUS_INSPECTOR=0 proteus`.

Для строгого token smoke откройте:

```text
http://127.0.0.1:1421/?session=<PROTEUS_SESSION_TOKEN>
```

## Граница

- `clients/inspector` владеет config/architecture views и может развиваться
  отдельно от ежедневного chat loop;
- `clients/web` остаётся чатовым клиентом;
- оба клиента используют HTTP app-server boundary и локальные serde DTO, не
  импортируя runtime internals из `proteus-core`;
- Config builder пишет только `[modules]` и `[module_config]` через
  `POST /config/builder`; provider profiles, secrets и `tools.enabled` остаются
  отдельными config surfaces;
- Mermaid грузится только здесь, чтобы chat bundle не тянул architecture
  dependencies.

Проверяйте inspector отдельной командой:

```bash
cargo check --manifest-path clients/inspector/Cargo.toml --target wasm32-unknown-unknown
```
