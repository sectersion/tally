export type Doc = { scheme: string; fsPath: string; isUntitled: boolean };

export function isTrackable(d: Doc): boolean {
  return d.scheme === "file" && !d.isUntitled;
}

// No file content, no branch, no project name: the daemon maps the path to a registered repo itself.
export function heartbeat(
  d: Doc,
  o: { isWrite: boolean; lineno: number; cursorpos: number; lines: number; language: string; nowMs: number; version: string },
): object {
  return {
    entity: d.fsPath,
    type: "file",
    time: o.nowMs / 1000,
    is_write: o.isWrite,
    lineno: o.lineno,
    cursorpos: o.cursorpos,
    lines: o.lines,
    language: o.language,
    plugin: `tally-vscode/${o.version}`,
  };
}

// The api key and file paths go only to the local daemon: plain http on a loopback host, nothing else.
export function isLoopbackUrl(s: string): boolean {
  try {
    const u = new URL(s);
    return u.protocol === "http:" && !u.username && !u.password && ["127.0.0.1", "localhost", "[::1]"].includes(u.hostname);
  } catch {
    return false;
  }
}
