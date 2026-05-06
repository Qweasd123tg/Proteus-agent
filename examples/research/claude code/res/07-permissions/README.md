# Permission Pipeline

## Главный вывод

Permission layer в Claude Code состоит из нескольких разных этапов:
- сборка `ToolPermissionContext`
- загрузка rules из CLI, settings и disk
- фильтрация tools до `allowed tools`
- runtime permission check перед `tool.call()`

Если смешать эти этапы в одну схему, получится неверная картина.  
Ниже я разложил их отдельно.

## Ключевые файлы

- `src/types/permissions.ts`
- `src/utils/permissions/PermissionMode.ts`
- `src/Tool.ts`
- `src/utils/permissions/permissionSetup.ts`
- `src/utils/permissions/permissions.ts`
- `src/utils/permissions/PermissionUpdate.ts`
- `src/tools.ts`
- `src/utils/toolPool.ts`
- `src/hooks/useCanUseTool.tsx`
- `src/services/tools/toolExecution.ts`

## Какие есть permission modes

### Пользовательские режимы

Из `src/types/permissions.ts` и `src/utils/permissions/PermissionMode.ts`:
- `default`
- `acceptEdits`
- `bypassPermissions`
- `plan`
- `dontAsk`

### Внутренние режимы

- `auto` - ant-only, появляется только при `TRANSCRIPT_CLASSIFIER`
- `bubble` - внутренний режим, не предназначен как внешний пользовательский режим

### Важное различие

- `ExternalPermissionMode` не включает `auto` и `bubble`
- `isExternalPermissionMode()` это явно отражает
- `toExternalPermissionMode()` может схлопывать внутренние значения в внешний режим

## Как строится `ToolPermissionContext`

`ToolPermissionContext` описан в [src/Tool.ts](/home/qweasd123tg/Code/Agent%20/Analys/claude/claude-code-src/src/Tool.ts).

Ключевые поля:
- `mode`
- `additionalWorkingDirectories`
- `alwaysAllowRules`
- `alwaysDenyRules`
- `alwaysAskRules`
- `isBypassPermissionsModeAvailable`
- `isAutoModeAvailable`
- `strippedDangerousRules`
- `shouldAvoidPermissionPrompts`
- `awaitAutomatedChecksBeforeDialog`
- `prePlanMode`

Сборка стартует в `initializeToolPermissionContext()`:
- парсятся `--allowed-tools`, `--disallowed-tools`, `--tools`
- при `baseToolsCli` автоматически вычисляются инструменты, которые нужно запретить
- подтягиваются rules из disk/settings
- вычисляется availability для `bypassPermissions`
- для `auto` дополнительно проверяются dangerous rules и classifier gate
- добавляются дополнительные рабочие директории через `--add-dir` и settings

Потом контекст может быть модифицирован:
- `applyPermissionUpdate()`
- `applyPermissionUpdates()`
- `stripDangerousPermissionsForAutoMode()`
- `restoreDangerousPermissions()`
- `prepareContextForPlanMode()`
- `transitionPlanAutoMode()`

## Где registered tools превращаются в allowed tools

### Базовая регистрация

`src/tools.ts` собирает полный registry tools:
- built-in tools
- feature-gated tools
- env-gated tools
- platform-gated tools
- some ant-only tools

### Первый фильтр

`getTools(permissionContext)`:
- отбрасывает tools, которые запрещены deny rules
- в simple mode возвращает только узкий набор primitives
- в REPL mode скрывает primitive tools от прямого использования
- в конце фильтрует по `tool.isEnabled()`

### Смешивание с MCP

`src/utils/toolPool.ts` и `assembleToolPool()`:
- берут built-in tools через `getTools()`
- добавляют MCP tools
- режут MCP tools по deny rules
- дедуплицируют по имени
- при coordinator mode дополнительно фильтруют по allowlist

### Вывод

С точки зрения документации полезно разделять:
- `registered tools` - что вообще существует в registry
- `allowed tools` - что прошло filters и visible to model
- `executed tools` - что реально дошло до execution

## Где фактически проверяется разрешение перед tool call

Фактический enforcement происходит в `src/services/tools/toolExecution.ts`.

Пайплайн там такой:
1. `runToolUse()` находит tool по имени
2. `checkPermissionsAndCallTool()` валидирует input через zod
3. выполняет `tool.validateInput()`
4. запускает `runPreToolUseHooks()`
5. вызывает `resolveHookPermissionDecision()`
6. внутри этого пути используется `canUseTool`
7. `canUseTool` опирается на `hasPermissionsToUseTool()`
8. если решение не `allow`, tool не вызывается
9. если `allow`, только тогда доходит до `tool.call()`

Самое важное:
- `canUseTool` вызывается через `useCanUseTool()` из `src/hooks/useCanUseTool.tsx`
- там `hasPermissionsToUseTool()` превращается в интерактивный диалог, classifier flow или headless auto-deny
- в headless/async-agent сценариях permission prompts могут быть заменены hooks или автоматическим deny
- `onChangeAppState.ts` не делает enforcement; он только синхронизирует состояние и metadata наружу

## Схема pipeline

```mermaid
flowchart TD
  modes["Permission modes"]
  init["initializeToolPermissionContext()"]
  ctx["ToolPermissionContext"]
  registry["tools.ts registry"]
  filtered["allowed tools"]
  pool["toolPool.ts"]
  useCan["useCanUseTool()"]
  core["hasPermissionsToUseTool()"]
  exec["toolExecution.ts"]
  prehooks["PreToolUse hooks"]
  call["tool.call()"]

  modes --> init
  init --> ctx
  ctx --> registry
  registry --> filtered
  filtered --> pool
  pool --> exec
  exec --> prehooks
  prehooks --> useCan
  useCan --> core
  core --> call
```

## Rule-based vs runtime enforcement

`checkRuleBasedPermissions()`:
- проверяет deny rules
- проверяет ask rules
- делегирует tool-specific `checkPermissions()`
- уважает bypass-immune safety checks
- **не** запускает auto classifier, mode transforms и hooks

`hasPermissionsToUseTool()`:
- делает все то же, что rule-based слой
- затем учитывает mode-based behavior
- затем может запустить auto classifier
- затем учитывает headless behavior
- затем возвращает финальный `allow / ask / deny`

## Где interactive и headless отличаются

### Interactive

В `src/hooks/useCanUseTool.tsx`:
- `ask` превращается в UI dialog flow
- `handleInteractivePermission()` может показать confirm dialog
- `setToolPermissionContext()` обновляет React/AppState

### Headless

В `useCanUseTool.tsx` и `permissions.ts`:
- если `shouldAvoidPermissionPrompts` включен, flow уходит в hooks
- если hooks не решили вопрос, tool auto-denied
- это защищает SDK/worker сценарии от зависания на UI prompt

### Важно

`src/cli/print.ts` и `src/QueryEngine.ts` используют тот же core permission logic, но без полноценно интерактивного UI-пути.

## Практические замечания

- `permissionSetup.ts` это не только CLI parsing, а еще gate-checking и transform layer.
- `permissions.ts` это core enforcement engine.
- `PermissionUpdate.ts` отвечает за применение и persistence updates.
- `ToolPermissionContext` не равен `AppState`, но `AppState.toolPermissionContext` хранит его runtime-снимок.
- Для диаграмм стоит отдельно рисовать:
  - setup
  - tool filtering
  - runtime enforcement
  - mode transitions
  - headless fallback
