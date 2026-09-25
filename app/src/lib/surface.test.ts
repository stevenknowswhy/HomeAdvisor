import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { INPUT_COMMANDS, READ_COMMANDS } from "./ipc";

// The frontend's half of the IPC surface audit (the authoritative half lives
// in `app/src-tauri/src/audit.rs`): the webview can reach the core only
// through `ipc.ts`, and `ipc.ts` offers only the audited read commands. A
// write or delete path added anywhere in the frontend fails here.

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      // Test support never ships: it may fake the boundary by design.
      if (entry.name === "test-support") return [];
      return sourceFiles(path);
    }
    // Test files never ship either; they may legitimately discuss forbidden
    // names.
    return entry.name.endsWith(".test.ts") ? [] : [path];
  });
}

const sources = sourceFiles(join(process.cwd(), "src"));
const offenders = (pattern: RegExp): string[] =>
  sources.filter((path) => pattern.test(readFileSync(path, "utf8")));

describe("the frontend IPC surface", () => {
  it("mirrors the backend's audited read commands, and only those", () => {
    // Keep in lockstep with `app_commands!` in `app/src-tauri/src/commands.rs`
    // — the Rust surface audit is authoritative; this pins the mirror.
    // READ_COMMANDS: managed-state-only commands (no webview input).
    expect([...READ_COMMANDS]).toEqual([
      "daily_recommendations",
      "privacy_status",
      "get_household",
    ]);
  });

  it("mirrors the backend's audited input commands, and only those", () => {
    // INPUT_COMMANDS: every command whose only webview-controlled
    // parameter is a named `*Input` payload struct — the onboarding
    // writes, and the receipts screen's pagination cursor.
    expect([...INPUT_COMMANDS]).toEqual([
      "egress_receipts",
      "create_household",
      "update_household",
      "add_member",
      "list_members",
      "create_goal",
      "update_goal",
      "delete_goal",
      "list_goals",
    ]);
  });

  it("keeps invoke reachable only from ipc.ts", () => {
    expect(
      offenders(/\binvoke\b/).map((path) => path.split("/").pop()),
    ).toEqual(["ipc.ts"]);
  });

  it("holds no raw SQL in any webview source", () => {
    const sql = offenders(
      /\b(INSERT\s+INTO|DELETE\s+FROM|UPDATE\s+\w+\s+SET)\b/i,
    );
    expect(sql).toEqual([]);
  });
});
