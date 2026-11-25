// Pure: the workspace folders Tally registers. Only local `file` folders; remote and virtual ones have no repo on this machine.
export function fileFolders(folders: readonly { scheme: string; fsPath: string }[] | undefined): string[] {
  return (folders ?? []).filter((f) => f.scheme === "file" && f.fsPath).map((f) => f.fsPath);
}
