# Browser Runtime Migration Plan

## Goal

Replace the current `Node + Playwright sidecar` browser control path with a native stack:

- `chromiumoxide` for browser lifecycle and CDP actions
- `enigo` for low-level keyboard/mouse fallback
- platform UI helper abstraction for semantic desktop UI control
  - macOS: `Swift AX helper`
  - Windows: `UIA helper`
  - Linux: no UI helper for now, command-line runtime only

The Tauri/tray/settings UI stays unchanged.

## Why

Current runtime is stable enough, but package size is dominated by the embedded Node runtime.

Current macOS app size breakdown:

- Rust tray binary: about 11 MB
- Embedded `node`: about 108 MB
- `node_modules`: about 14 MB

Main objective:

- remove embedded Node
- keep product behavior stable
- reduce moving parts in browser execution path
- keep room for macOS-native automation without forcing everything into browser CDP

## Current Boundaries

### Existing Rust browser crate

Current action runtime lives in `crates/browser/src/lib.rs`.

Responsibilities today:

- action dispatch entry: `execute_action(config, action, payload)`
- browser actions:
  - `browser.launch`
  - `browser.goto`
  - `browser.read`
  - `browser.extract`
  - `browser.click`
  - `browser.type`
  - `browser.screenshot`
- file actions:
  - `file.list`
  - `file.stat`
  - `file.read_text`
  - `file.open`
- local actions:
  - `screen.capture`
  - `system.info`

### Existing sidecar dependency

Current browser actions call a Node sidecar:

- `sidecar/playwright-agent.mjs`
- process lifecycle managed through `BrowserSidecar`
- browser mode/preference switching handled in sidecar process

### Existing platform crate

`crates/platform/src/lib.rs` already owns:

- permission checks
- permission settings deeplink handling
- relaunch behavior
- launch-agent helpers

This is the correct place to keep permission detection and app-level platform operations.

## Target Architecture

### 1. `crates/browser`

Keep this crate as the public action execution surface, but replace internal browser backend.

Responsibilities:

- keep `execute_action()` as stable entrypoint
- route `browser.*` actions to a Rust browser session manager
- keep `file.*`, `screen.capture`, `system.info` here for now
- optionally use `enigo` only as fallback for browser-native actions that CDP cannot do robustly

### 2. New browser session manager

Add a Rust-managed browser runtime layer inside `crates/browser`, for example:

- `browser/runtime.rs`
- `browser/session.rs`
- `browser/actions.rs`

Responsibilities:

- launch/connect Chrome/Edge using `chromiumoxide`
- preserve current config concepts:
  - `browser_preference`
  - `browser_mode`
- own browser lifecycle:
  - start
  - reuse
  - shutdown
  - force restart when preference/mode changes
- avoid zombie processes and keep cleanup explicit

### 3. New platform UI helper bridge

Add a platform-specific semantic UI helper outside the Rust main binary.

Recommended locations:

- `helpers/macos-ax-helper/`
- `helpers/windows-uia-helper/` (planned)

Responsibilities:

- query Accessibility tree or equivalent platform UI tree
- locate windows and focused apps
- semantic click/type/activate operations where platform UI automation is a better fit than raw events
- app/window focus recovery for headed browser mode
- permission-oriented diagnostics

Important boundary:

- this helper layer is only for platform semantic UI control
- shared browser automation remains in `chromiumoxide`
- shared low-level input fallback remains in `enigo`

Bridge options:

- JSON over stdin/stdout
- local Unix socket
- simple request/response CLI

Recommended first version:

- stdin/stdout JSON, same style as the current Node sidecar, because it keeps orchestration simple

### 4. `enigo` fallback layer

Use `enigo` only for low-level event simulation, not as the primary semantic engine.

Responsibilities:

- emergency click/type fallback when CDP target is unavailable but the browser window is focused
- future support for limited non-browser keyboard/mouse automation

Non-goal:

- do not build desktop semantic automation purely on `enigo`


### 5. Cross-platform split

Shared across macOS and Windows:

- `chromiumoxide`
- `enigo`
- Rust action protocol

Platform-specific only for semantic desktop UI control:

- macOS: Swift AX helper
- Windows: UIA helper
- Linux: no UI helper in current scope

## Action Mapping

### Browser actions to keep

These should remain part of the public protocol:

- `browser.launch`
- `browser.goto`
- `browser.read`
- `browser.extract`
- `browser.click`
- `browser.type`
- `browser.screenshot`

### Browser actions implementation target

- `browser.launch`
  - ensure browser process and at least one page exist
- `browser.goto`
  - page navigation through CDP
- `browser.read`
  - navigate and extract text/html from page or selector
- `browser.extract`
  - extract from current page or optional url
- `browser.click`
  - first try DOM click via CDP selector resolution
  - only fall back to AX/enigo when necessary
- `browser.type`
  - first try DOM fill/type via CDP
  - fall back only if input target is outside page DOM scope
- `browser.screenshot`
  - page screenshot through browser backend when possible

### Non-browser actions

These can stay mostly unchanged initially:

- `file.*`
- `screen.capture`
- `system.info`

## Phase Plan

### Phase 1: Internal backend abstraction

Goal:

- stop hard-coding Node sidecar inside `execute_browser_action()`

Tasks:

- introduce a `BrowserBackend` trait or equivalent internal abstraction
- move current sidecar path behind `PlaywrightBackend`
- keep behavior unchanged

Deliverable:

- browser backend becomes swappable without changing runtime protocol

### Phase 2: Add `chromiumoxide` backend

Goal:

- get `browser.launch/goto/read/extract/screenshot` working without Node

Tasks:

- add crate dependency
- implement browser startup and page reuse
- implement mode/preference restart rules
- preserve current config-driven behavior

Deliverable:

- read-only browser workflow works natively in Rust

### Phase 3: Add DOM interaction support

Goal:

- support `browser.click` and `browser.type` through CDP first

Tasks:

- selector lookup
- wait/timeout behavior
- input actions
- parity with current success/error payload format

Deliverable:

- common web interactions no longer depend on Node

### Phase 4: Introduce platform UI helper abstraction

Goal:

- support window focus and semantic desktop UI automation where browser CDP is not enough

Tasks:

- Rust-side helper abstraction
- macOS helper process skeleton
- Windows helper placeholder
- request/response protocol
- app activate/focus/window query methods
- permission diagnostics method

Initial helper commands:

- `app.activate`
- `window.focus`
- `ax.status`
- `ax.tree` (debug-only)

Deliverable:

- stable platform UI helper abstraction without pushing platform UI semantics into browser backend

### Phase 5: Optional `enigo` fallback

Goal:

- support limited low-level fallback actions

Tasks:

- mouse click at coordinates
- typing fallback
- guarded usage only when browser/AX path requests it

Deliverable:

- minimal fallback path, not default path

### Phase 6: Remove Node sidecar packaging

Goal:

- drop embedded Node and `node_modules` from app build

Tasks:

- remove sidecar packaging from build scripts
- remove runtime Node resolution path
- shrink distribution artifacts

Deliverable:

- app package size falls dramatically

## Stability Rules

These rules should be preserved during migration:

- config changes to browser mode/preference must immediately shut down current browser runtime
- runtime must never leave obvious zombie browser processes on config switch or app quit
- permission failures must remain user-readable and route to permission guide UI
- action protocol shape must stay compatible with backend PHP scheduler/runtime expectations

## Suggested Repository Changes

### New files or modules

- `crates/browser/src/backend/mod.rs`
- `crates/browser/src/backend/playwright.rs`
- `crates/browser/src/backend/chromiumoxide.rs`
- `crates/browser/src/runtime.rs`
- `helpers/macos-ax-helper/README.md`
- `helpers/macos-ax-helper/` Swift package or Xcode project

### Existing files that will change

- `crates/browser/src/lib.rs`
- `scripts/build-macos-app.sh`
- packaging docs in `README.md`

## Recommendation

Recommended execution order:

1. introduce internal backend abstraction
2. land `chromiumoxide` read-only path
3. land `chromiumoxide` interaction path
4. add Swift AX helper for macOS semantic control
5. remove Node packaging only after parity is verified

This keeps shipping risk low while still moving toward the smaller and more native runtime.
