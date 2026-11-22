# @wae/open-eda-ui

The OpenEDA Studio window: a static VMZ interface served by the desktop host.

```bash
pnpm --filter @wae/open-eda-ui run build   # vmz build --profile static
pnpm --filter @wae/open-eda-ui run serve   # local preview
```

The page talks to the native host over `chrome.webview.postMessage` JSON-RPC.
Without a WebView host it shows an empty canvas and a "Preview mode — native
window unavailable" status; it never fabricates a design. The bundled build is
staged into the `open-eda` package and opened with `open-eda gui`.

Studio behavior and the JSON-RPC contract are documented in
[architecture.md](../../../documentation/architecture.md).

## License

Mozilla Public License 2.0.
