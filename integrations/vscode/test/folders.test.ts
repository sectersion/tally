import { expect, test } from "bun:test";
import { fileFolders } from "../src/folders";

test("only file-scheme folders are registered, paths untouched", () => {
  const got = fileFolders([
    { scheme: "file", fsPath: "C:/work/My Code é" },
    { scheme: "vscode-remote", fsPath: "/home/x/repo" },
    { scheme: "vscode-vfs", fsPath: "/a" },
    { scheme: "file", fsPath: "/tmp/ünï code" },
  ]);
  expect(got).toEqual(["C:/work/My Code é", "/tmp/ünï code"]);
});

test("no workspace is an empty list", () => {
  expect(fileFolders(undefined)).toEqual([]);
  expect(fileFolders([])).toEqual([]);
});
