# Windows UIA Helper

This directory is the planned Windows companion to the macOS Swift AX helper.

## Scope

- expose the same stdin/stdout JSON protocol as the macOS helper
- provide Windows UI Automation based semantic desktop UI control
- keep browser automation itself out of this helper

## Shared boundary

Shared across macOS and Windows:

- `chromiumoxide` for browser automation
- `enigo` for low-level input fallback
- Rust action protocol

Windows-only in this helper:

- app activation
- window focus
- semantic element lookup
- UI Automation tree diagnostics

## Planned first commands

- `ax.status`
- `app.activate`
- `window.focus`
- `ax.tree`
