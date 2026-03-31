# macOS AX Helper

This helper will be the macOS-native semantic automation bridge for Dux AI Node.

## Scope

- Accessibility tree inspection
- App activation and focus
- Window focusing
- Semantic element lookup for non-browser desktop control
- Permission diagnostics for Accessibility-related flows

## Non-goals

- It is not a replacement for browser CDP automation.
- It is not the primary implementation for web page DOM control.

## Planned transport

First version will use JSON request/response over stdin/stdout, matching the current sidecar style.

Example request shape:

```json
{
  "id": "req-1",
  "action": "app.activate",
  "payload": {
    "bundle_id": "com.google.Chrome"
  }
}
```

Example response shape:

```json
{
  "id": "req-1",
  "ok": true,
  "result": {
    "summary": "application activated"
  }
}
```

## First commands

- `ax.status`
- `ui.status`
- `app.activate`
- `window.focus`
- `ui.find`
- `ui.read`
- `ui.write`
- `ui.invoke`
- `ui.click`
- `ui.type_native`
- `ui.keypress`
- `ui.tree`
- `ax.tree` (debug only)

## Integration target

This helper will be called from `crates/browser` as the macOS-native companion to:

- `chromiumoxide` browser control
- `enigo` low-level fallback input
