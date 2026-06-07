# Changelog

All notable changes to UsageDock are documented here.

## [0.2.10] - 2026-06-06

- Added Google's Antigravity (Gemini/AI IDE) quota tracking provider, supporting both local language server discovery (no authentication needed) and Cloud Code API credentials fallback.
- Re-architected and compacted the tray UI layout to recover ~102px (~20% of vertical space) for provider cards.
- Moved auto-refresh and check-for-updates controls into a new slide-down settings panel accessed via a gear icon.
- Polished layout details: tightened logo-to-title gap, narrowed the status pill, and removed duplicate reset timing labels on metrics.

## [0.2.9] - 2026-04-15

- Refined provider card density, reset-text readability, and multi-metric spacing for a cleaner tray view.
- Added a smooth settings-panel close animation and a keyboard `R` shortcut for refresh-all while focused.
- Improved per-card refresh affordance behavior and fixed long provider error messages so they wrap cleanly inside the card.

## [0.2.8] - 2026-04-15

- Compressed the tray header into a single tighter row and moved refresh settings into a top settings panel.
- Reduced duplicate reset copy in provider cards and cleaned up header spacing and status density.
- Fixed the custom interval menu so it opens downward from the top settings row and layers cleanly above provider cards.

## [0.2.7] - 2026-04-12

- Added Cursor free-tier usage rendering so included usage percentages show instead of a generic "No usage data" state.
- Updated the tray header subtitle to the shorter and clearer line: `Local AI coding usage at a glance`.

## [0.2.6] - 2026-04-11

- Hardened provider integration paths by removing untrusted executable lookup fallbacks for `gh`, `ps`, and `powershell.exe`.
- Parameterized SQLite reads in the touched providers to avoid string-built queries.
- Preserved the current Windsurf local-LS behavior while keeping the stricter endpoint-selection fix in place.

## [0.2.4] - 2026-03-29

- Merged the current `dev` branch into `main` for the public release line.
- Shipped the hardened app configuration, updater plumbing, and provider integration fixes on `main`.
- Added open-source repository basics including MIT licensing, contribution guidance, security policy, changelog, and GitHub templates.

## [0.2.3] - 2026-03-29

- Added updater artifact workflow fixes for Windows and Linux release jobs.
- Hardened the Tauri app configuration and removed unused opener capability.
- Kept the packaged app startup stable with a minimal updater plugin configuration.

## [0.2.2] - 2026-03-29

- Added in-app updater support and release workflow wiring.
- Added updater setup documentation for signing keys and release configuration.

## [0.2.1] - 2026-03-29

- Refined provider card copy and reset timing visibility.
- Updated app screenshot handling on the landing page and follow-up packaging fixes.

## [0.2.0] - 2026-03-25

- Shipped the first broader public UsageDock release milestone.
- Refreshed the tray UI and branding.
- Added provider grouping, auto-refresh controls, and custom interval selection.
- Improved Windsurf detection and Windows tray behavior.
- Added single-instance tray behavior.
