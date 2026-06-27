## Frontend tasks

When doing frontend design tasks, avoid generic, overbuilt layouts.

**Use these hard rules:**
- One composition: The first viewport must read as one composition, not a dashboard (unless it's a dashboard).
- Brand first: On branded pages, the brand or product name must be a hero-level signal, not just nav text or an eyebrow. No headline should overpower the brand.
- Brand test: If the first viewport could belong to another brand after removing the nav, the branding is too weak.
- Typography: Use expressive, purposeful fonts and avoid default stacks (Inter, Roboto, Arial, system).
- Background: Don't rely on flat, single-color backgrounds; use gradients, images, or subtle patterns to build atmosphere.
- Cards: Default: no cards. Never use cards in the hero. Cards are allowed only when they are the container for a user interaction. If removing a border, shadow, background, or radius does not hurt interaction or understanding, it should not be a card.
- One job per section: Each section should have one purpose, one headline, and usually one short supporting sentence.
- Real visual anchor: Imagery should show the product, place, atmosphere, or context. Decorative gradients and abstract backgrounds do not count as the main visual idea.
- Reduce clutter: Avoid pill clusters, stat strips, icon rows, boxed promos, schedule snippets, and multiple competing text blocks.
- Use motion to create presence and hierarchy, not noise. Ship at least 2-3 intentional motions for visually led work.
- Color & Look: Choose a clear visual direction; define CSS variables; avoid purple-on-white defaults. No purple bias or dark mode bias.
- Ensure the page loads properly on both desktop and mobile.
- For React code, prefer modern patterns including useEffectEvent, startTransition, and useDeferredValue when appropriate if used by the team. Do not add useMemo/useCallback by default unless already used; follow the repo's React Compiler guidance.

Exception: If working within an existing website or design system, preserve the established patterns, structure, and visual language.

## Devmeter context

`devmeter` is the main open-source product. It is a Tauri v2 tray app called `UsageDock` that shows local AI coding tool usage in a compact dock-style popup.

### Product truth

- The app is tray-first and should feel like a local utility, not a dashboard.
- Supported provider integrations include Cursor, Claude, GitHub Copilot, Codex, and Windsurf.
- Key product behavior already shipped:
  - active providers surface first
  - unavailable providers stay in a collapsible section
  - configurable auto refresh with manual per-provider refresh
  - in-app updater support for packaged releases
  - single-instance tray behavior
- The current tray UI direction is compact and information-dense:
  - single-row header with logo, product name, status, settings, and refresh
  - auto-refresh controls live in the settings panel, not the footer
  - provider cards are tightened for tray use
  - reset text should be readable but visually quiet
  - per-card refresh controls are de-emphasized and should not dominate the card
  - long provider error messages must wrap inside the card
  - keyboard `R` refreshes all providers while the tray window is focused
- The repo includes open-source basics already added:
  - `LICENSE`
  - `README.md`
  - `CONTRIBUTING.md`
  - `SECURITY.md`
  - `CODE_OF_CONDUCT.md`
  - `CHANGELOG.md`
  - GitHub issue templates and PR template

### Release and versioning

- The latest released app version in recent work is `v0.2.9`.
- Release from `main`, not from `dev` or feature branches, unless there is an explicit reason.
- The GitHub Actions release workflow is in `.github/workflows/release.yml`.
- Releases are triggered by pushing a tag matching `v*`.
- Before tagging, keep versions aligned in:
  - `package.json`
  - `package-lock.json`
  - `src-tauri/Cargo.toml`
  - `src-tauri/Cargo.lock`
  - `src-tauri/tauri.conf.json`
- Build commands used before release:
  - `npm run build`
  - `cargo check --manifest-path src-tauri/Cargo.toml`
  - `npx tauri build --bundles nsis`
- Updater-enabled releases depend on these GitHub secrets:
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  - `TAURI_UPDATER_PUBLIC_KEY`
- The updater endpoint is:
  - `https://github.com/anshuman55a/UsageDock/releases/latest/download/latest.json`

### Important implementation notes

- `devmeter` currently bundles Windows `nsis` locally, while the GitHub release workflow builds Windows and Linux artifacts.
- The Tauri updater plugin needs a real `plugins.updater` block present in config. A minimal placeholder exists in `src-tauri/tauri.conf.json` so the app does not fail at startup in non-updater local builds.
- The Tauri bundle identifier currently ends with `.app` (`com.usagedock.app`). This is tolerated on Windows but Tauri warns about it for macOS.
- The opener plugin was intentionally removed. Do not re-add it unless the app actually needs external link opening.
- The native `get_providers` command was intentionally removed from the invoke surface.
- The README was expanded for open-source use and should include Windows build prerequisites such as Visual Studio Build Tools 2022, the C++ workload, Windows SDK, and WebView2.
- Keep maintainer-only updater secrets and signing details out of the main README; point to `UPDATER_SETUP.md` instead.

### Provider-specific notes

- Cursor free-tier usage is supported. Cursor may return `planUsage.totalPercentUsed`, `autoPercentUsed`, `apiPercentUsed`, and `billingCycleEnd` without paid-plan dollar limits; this should render as included usage instead of `No usage data`.
- Copilot currently depends on GitHub CLI auth. `gh auth login` must be available for the provider to work.
- Codex and Copilot reset timing surfaced in the UI after backend/frontend timestamp alignment fixes.
- Claude is expected to work from code review, but live validation requires a real Claude-authenticated machine.

### Windsurf notes

Windsurf is the most fragile provider and has specific local behavior that must be preserved.

- It reads auth state from Windsurf's local `state.vscdb`.
- On Windows, the language server process can expose multiple ports.
- Do not select a candidate port just because a request can be sent to it.
- The correct endpoint selection rule is: only accept a candidate when `GetUserStatus` succeeds on that port and scheme.
- A previous pre-probe against `GetUnleashData` was incorrect and filtered out the real working local endpoint on this machine.
- On this machine, the real working Windsurf LS endpoint was observed at:
  - `http://127.0.0.1:55547`
- Trusted HTTPS did not work here, so plain HTTP localhost fallback is required for functionality.
- The safer compromise is:
  - try trusted HTTPS first
  - then fall back only on loopback-hosted insecure local transport
  - keep candidate selection strict by requiring a valid `GetUserStatus` response
- Do not reintroduce a loose `send().is_ok()`-style probe, and do not rely on `GetUnleashData` as the deciding probe for endpoint selection.

### Current security status

- The low-risk hardening pass is already applied:
  - trusted executable lookup is used for `gh`, `ps`, and `powershell.exe`
  - bare `PATH` fallback was removed
  - SQLite reads in the touched providers were parameterized
- The security review is not fully closed:
  - the Windsurf localhost fallback still allows invalid-cert HTTPS and plain HTTP on loopback
  - this remains intentionally unresolved because the local Windsurf installation observed in this workspace requires the HTTP fallback to function
- Treat the Windsurf transport issue as an open security tradeoff, not as a finished fix.

### Microsoft Store and MSIX context

- The current GitHub/direct-download Windows package is an NSIS installer, not MSIX.
- Unsigned NSIS installers can still show SmartScreen `Windows protected your PC` and `Publisher: Unknown publisher`.
- Adding `bundle.publisher` metadata alone does not fix SmartScreen. `Unknown publisher` is fixed by trusted Authenticode signing for EXE/MSI installers or by Store/MSIX package signing for Store-hosted package flows.
- Tauri updater signing is separate from Windows code signing and does not affect SmartScreen.
- The desired Store direction is Store-hosted MSIX/package identity if the goal is to avoid the standalone unsigned installer warning for Store users.
- Tauri does not directly emit a final Store-ready MSIX in the current repo setup. The practical path is:
  - build the normal Tauri release executable/installer
  - use Microsoft MSIX Packaging Tool or Windows SDK tooling to create MSIX
  - inspect and adjust `Package.appxmanifest`
  - validate provider file access under packaged app identity
  - submit the MSIX through Partner Center
- MSIX capabilities such as `runFullTrust` and `broadFileSystemAccess` belong in `Package.appxmanifest`, not in Tauri `src-tauri/capabilities/*.json`.
- Start MSIX testing with `runFullTrust`. Do not add `broadFileSystemAccess` unless provider file reads fail under MSIX and work under normal Win32 execution.
- UsageDock has a strong Store certification explanation:
  - it reads local auth state created by tools already installed on the user's machine
  - users do not paste provider tokens into UsageDock
  - no provider credentials or usage data are sent to a UsageDock-operated backend
  - unavailable providers should appear as unavailable if their local tools are not installed/authenticated
- Privacy policy text should emphasize local-first operation, no UsageDock account, no hosted backend for core usage tracking, local preference storage, and update checks that do not send provider credentials.
- Certification notes must mention that the app runs from the system tray and may not appear as a normal taskbar window after launch.
- The current checkout may not contain MSIX helper files; check the branch before assuming `MICROSOFT_STORE_SUBMISSION.md`, `scripts/package-msix.ps1`, or `src-tauri/tauri.microsoftstore.conf.json` exist.

<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**IMPORTANT: This project has a knowledge graph. ALWAYS use the
code-review-graph MCP tools BEFORE using Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives
you structural context (callers, dependents, test coverage) that file
scanning cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
| ------ | ---------- |
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.
