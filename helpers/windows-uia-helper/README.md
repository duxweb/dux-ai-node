# Windows UIA Helper

This directory is the Windows companion to the macOS Swift AX helper.

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

## Current commands

- `ui.status`
- `ax.status`
- `app.activate`
- `window.focus`
- `ui.tree`
- `ax.tree`

## Runtime

- current implementation ships as `dux-node-windows-uia-helper.ps1`
- transport is JSON request/response over stdin/stdout
- packaged by `scripts/build-windows.ps1`

## Notes

- current Windows helper focuses on app/window activation and top-level window inspection
- deeper semantic element lookup for specific desktop clients will continue to grow on top of this helper
