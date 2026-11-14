# open-eda-ui

VMZ static UI for the OpenEDA Studio window. `vmz build . --profile static`
emits `dist/static`, which the repository staging step copies to
`projects/packages/open-eda/dist/gui` for the native WebView host.

```bash
pnpm --filter @wae/open-eda-ui run check   # vmz check
pnpm --filter @wae/open-eda-ui run build   # vmz build --profile static
pnpm --filter @wae/open-eda-ui run serve   # local preview
```

The page talks to the host over `chrome.webview.postMessage` JSON-RPC
(`ready`, `capabilities`, `design.create`, `design.open`, `design.save`,
`design.close`, `close`). Without a WebView host the studio shows an empty
canvas and a "Preview mode — native window unavailable" status; it never
fabricates a design.
