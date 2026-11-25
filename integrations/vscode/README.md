# Tally for VS Code

Sends a small heartbeat (file path, line, cursor, line count, language) to the Tally daemon on your own machine when you save, switch editors or type. At most one per file every 2 minutes, plus one on every save. No file content, branch or project name leaves the editor, and nothing goes anywhere except `127.0.0.1`.

On startup it looks for `tally` in `<tally home>/bin`, then on PATH, and runs `tally ensure --client vscode`. If `tally` is missing it offers an "Install Tally" button, which runs a bundled installer if this build has one; otherwise see `docs/extensions.md`. The extension never downloads anything itself.

Build: `bun install && bun test && bunx tsc --noEmit && bunx vsce package`. Works in VS Code and compatible forks that load VS Code extensions (Cursor, Windsurf, VSCodium: not tested).
