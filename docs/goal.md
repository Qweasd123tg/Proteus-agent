You are working in the repository:

https://github.com/Qweasd123tg/Proteus-agent

Goal:
Bring Proteus-agent to a solid v0 dogfood-ready state by closing the current narrow bottlenecks without expanding scope. The project is a Rust-first modular coding-agent harness with proteus-core, proteus-contracts, dylib plugins, an HTTP/SSE app-server boundary, and an experimental Leptos web client. The old TUI path has been removed. Do not revive TUI.

Primary objective:
Make the current web/app-server path safe, reliable, testable, and usable enough for one real dogfood coding task.

Important mindset:
This is not a UI polish task.
This is not a rewrite task.
This is not a “make it pretty” task.
This is a hardening + dogfood-readiness task.

Work rules:
1. First inspect the repository and current state before editing.
2. Read at minimum:
   - README.md
   - docs/dogfood-gate.md
   - crates/proteus-contracts/src/app_protocol.rs
   - crates/proteus-core/src/app_server/http.rs
   - crates/proteus-core/src/app_server/*.rs
   - clients/web/README.md
   - clients/web/src/**/*.rs
   - install.sh
   - Cargo.toml
3. Make small reviewable commits.
4. Do not make giant mixed commits.
5. Prefer fixing real bottlenecks over refactoring.
6. Do not add new large dependencies unless absolutely necessary.
7. Do not rewrite the web client architecture unless a smaller fix is impossible.
8. Do not add new product features outside the scope below.
9. Preserve existing public contracts unless a security fix requires a small compatible extension.
10. If you must make a protocol change, update proteus-contracts, core, web client, docs, and tests together.
11. Run relevant tests before each commit when practical.
12. At the end, run the full validation commands listed below.
13. If an environment problem prevents a command from running, document the exact command and exact error in the final report.

Create a branch:

codex/v0-dogfood-hardening

Expected commit style:
Use conventional commits, for example:
- fix(http): require local session token
- fix(web): constrain approval cache scopes
- fix(web): rerender tool activity state changes
- test(app-server): cover auth and approval cache regressions
- docs: update dogfood and web client runbooks

Do not commit broken formatting.
Do not leave TODOs for the main blockers.
Do not leave debug prints.
Do not log secrets or session tokens.

Priority P0 — local HTTP app-server safety:

The local HTTP/SSE app-server currently exposes powerful agent operations such as sending prompts, approvals, mode changes, cancellation, reload-tools, history/resume, and shutdown. Harden this boundary before dogfood.

Implement a minimal local session authentication model.

Requirements:
1. Generate or accept a per-server session token for the HTTP app-server.
2. Protect all non-trivial HTTP endpoints with this token:
   - /events
   - /send
   - /approval
   - /user-input
   - /cancel
   - /mode
   - /model
   - /reasoning
   - /effort
   - /config
   - /sessions
   - /history
   - /resume
   - /clear
   - /reload-tools
   - /shutdown
   - any similar app-control endpoint
3. It is okay to leave a minimal /health endpoint unauthenticated if one exists or is useful.
4. For SSE/EventSource, support token passing in a way browsers can actually use. EventSource cannot set arbitrary headers, so a query parameter token is acceptable for v0.
5. For POST/fetch requests from the web client, prefer an explicit header such as X-Proteus-Session or Authorization: Bearer <token>. Query token fallback is acceptable if it keeps the implementation simple.
6. Do not print the raw token in normal logs.
7. If the wrapper opens the web UI, pass the token to the web client in a minimal way, for example via URL query params. The web client should parse it and use it for API calls.
8. Avoid persisting the token in localStorage. In-memory state or sessionStorage is acceptable for v0 if needed.
9. Return 401 or 403 for missing/invalid token.
10. Add tests for:
    - missing token is rejected
    - invalid token is rejected
    - valid token is accepted
    - /events requires token
    - at least one mutating endpoint requires token

Also restrict CORS.

Requirements:
1. Remove wildcard Access-Control-Allow-Origin: * for the app-server.
2. Allow only explicit local origins required by the web client, such as:
   - http://127.0.0.1:<web_port>
   - http://localhost:<web_port>
3. Make the allowed origin configurable if the current wrapper/dev flow needs dynamic ports.
4. Validate Origin on browser requests.
5. Handle OPTIONS preflight correctly.
6. If a request has no Origin but has a valid token and comes from local tooling/curl, allow it unless there is an existing reason not to.
7. Add tests for:
   - bad Origin rejected
   - allowed Origin accepted
   - no wildcard CORS remains on protected app endpoints

Priority P0 — approval cache safety:

The app-server and web client must not allow broad cached approval for command-like or network-like tools.

Problem to prevent:
A user must not be able to approve one shell command with ToolInCwd and accidentally approve all future shell commands in that cwd.

Requirements:
1. Find all approval cache handling in core and web client.
2. Enforce this rule backend-side, not only in the UI:
   - For tool name "shell", requested ToolInCwd must be rejected or downgraded to ExactCall.
   - For tools whose safety is RunsCommands, Network, or Dangerous, ToolInCwd must be rejected or downgraded to ExactCall.
3. The web UI should also avoid presenting Tool/CWD as an option for such tools.
4. For safer write-like tools, ToolInCwd may remain available if current policy allows it.
5. Add regression tests for:
   - shell + ToolInCwd cannot create a broad cached approval
   - RunsCommands/Network/Dangerous + ToolInCwd cannot create broad cached approval
   - a safer tool can still use the intended allowed cache scope if applicable

Priority P0 — web tool card correctness:

Make sure the Leptos web client reliably updates tool activity cards.

Problem to check:
If keyed rendering uses only message id/text length/streaming, tool cards with empty text may not rerender when status changes from Running -> WaitingApproval -> Approved -> Done/Failed.

Requirements:
1. Inspect clients/web message rendering and keyed <For> usage.
2. Ensure tool messages rerender when any of these change:
   - tool status
   - result preview
   - approval state
   - error state
3. Prefer a small key fix or reactive state fix over a rewrite.
4. Add a small unit test if the web crate has suitable test structure. If not practical, add a focused code comment explaining why the key includes tool state.
5. Do not redesign the whole web client.

Priority P1 — dogfood reliability:

Make the web path enough for one real dogfood loop.

Required dogfood loop:
1. Start app-server and web client through the intended command/wrapper.
2. Send a prompt.
3. See model/tool activity.
4. See approval request.
5. Approve once.
6. Deny once.
7. Submit typed user input if requested.
8. Cancel an active turn.
9. Inspect config/session/history enough to understand what happened.
10. Preserve or expose event log/history enough for debugging.

Tasks:
1. Verify current /cancel behavior clears pending approvals and pending user inputs.
2. Verify web cancel button calls the right endpoint with auth token.
3. Verify user-input submission includes auth token and handles success/failure.
4. Verify approval submission includes auth token and handles success/failure.
5. Make error messages visible in the web UI when API calls fail due to auth, server down, malformed response, or denied request.
6. Do not make visual polish the priority. Plain readable errors are enough.

Priority P1 — config and startup clarity:

Reduce hidden state and startup confusion.

Requirements:
1. Verify single-file config path behavior:
   - ~/.config/Proteus-agent/configs/config.toml or the current intended path
2. Verify proteus init warns about mixed legacy config files in the same config directory.
3. Verify proteus doctor warns about mixed config files if applicable.
4. If already implemented, only fix gaps/bugs.
5. Make sure README and docs describe the real current config path.
6. Make sure commands use proteus, not old agent, except when explicitly documenting legacy migration.

Priority P1 — public docs cleanup:

The repository is public. Make the docs look coherent enough for outsiders.

Requirements:
1. Replace stale commands:
   - agent modules list -> proteus modules list
   - agent tools list -> proteus tools list
   - other stale agent commands unless intentionally marked legacy
2. README:
   - Do not say “cargo build --workspace” builds literally everything if clients/web is excluded from the workspace.
   - Change wording to “build core and plugins” or equivalent.
   - Add or verify separate web check command:
     cargo check --manifest-path clients/web/Cargo.toml --target wasm32-unknown-unknown
3. Rename “Web client migration” to something like:
   - “Experimental web client”
   - or “Web client”
4. Fix obvious markdown formatting issues:
   - headings must have blank lines after them
   - no accidentally collapsed paragraphs
   - no doubled words from search/replace accidents
5. Do not spend time rewriting research docs unless they are linked from README and look broken.
6. If citation artifacts like  appear in public-facing docs, either remove them, convert them to normal markdown links, or move the file out of the main docs path if appropriate.

Priority P1 — tests and validation:

Add or update tests where possible.

At minimum try to cover:
1. HTTP token auth.
2. CORS allowed/rejected origins.
3. Approval cache scope sanitization.
4. Cancel/shutdown behavior for pending approval/user-input if not already covered.
5. Web message rerender key or equivalent state logic if practical.
6. Config mixed-file warning if easy to test.

Validation commands to run:

cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

Then for web:

rustup target add wasm32-unknown-unknown

cargo check --manifest-path clients/web/Cargo.toml --target wasm32-unknown-unknown

If clippy -D warnings is too noisy because the repository already has warnings unrelated to your changes, do not blindly refactor the world. Instead:
1. Run clippy.
2. Fix warnings caused by your changes.
3. Document pre-existing warnings in the final report.

Manual smoke test if the environment supports it:

./install.sh
proteus init coding
proteus doctor
proteus

Then verify:
1. Web UI opens or can be opened manually.
2. Web client connects to app-server.
3. Unauthorized requests fail.
4. Authorized web client can send a prompt.
5. Approval/user-input/cancel work through the web UI.

Commit plan:

Commit 1:
fix(http): require local session token and restrict origins

Should include:
- token support
- CORS restriction
- protected endpoint checks
- web client token usage if needed for server connectivity
- tests for auth/CORS

Commit 2:
fix(approval): constrain broad cache scopes for command-like tools

Should include:
- backend enforcement
- web UI option hiding/downgrade
- tests

Commit 3:
fix(web): update tool cards when activity state changes

Should include:
- rerender/state fix
- small tests/comments if practical

Commit 4:
fix(web): surface app-server request failures

Should include:
- visible error messages for failed send/approval/user-input/cancel/config/history calls
- no large UI redesign

Commit 5:
docs: align public docs with Proteus web dogfood path

Should include:
- stale agent -> proteus command cleanup
- README build wording
- web client section rename
- markdown formatting fixes

Commit 6, only if needed:
test: add dogfood regression coverage

Should include:
- focused tests that did not fit cleanly into earlier commits

Final response format:
At the end, give me:

1. Commit list with hashes and one-line summaries.
2. What was fixed.
3. What tests were run and whether they passed.
4. What could not be tested and why.
5. Any remaining risks, ranked P0/P1/P2.
6. Exact commands I should run locally next.

Important non-goals:
- Do not revive TUI.
- Do not add desktop wrapper.
- Do not add WebSocket unless absolutely necessary.
- Do not add new agent features.
- Do not redesign plugin ABI.
- Do not rewrite the whole web app.
- Do not polish CSS beyond readability/error visibility.
- Do not hide failures.
- Do not merge unrelated refactors.
- Do not make one giant commit.

Success condition:
The repository should be safer and clearer, and I should be able to run one dogfood coding task through the web/app-server path without wondering whether the failure came from hidden config, unsafe approval semantics, broken web state updates, or unauthenticated localhost access.
