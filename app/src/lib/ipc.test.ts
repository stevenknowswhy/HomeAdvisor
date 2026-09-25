import { describe, expect, it, vi } from "vitest";

// The real backend boundary is mocked at its package seam so these tests
// observe exactly what the app would invoke.
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  errorMessage,
  fetchDailyRecommendations,
  fetchEgressReceipts,
  inputCommand,
  isInputCommand,
  isReadCommand,
  readCommand,
} from "./ipc";

describe("readCommand", () => {
  it("invokes an audited read command by name", async () => {
    invoke.mockResolvedValueOnce([]);
    await readCommand("daily_recommendations");
    expect(invoke).toHaveBeenCalledWith("daily_recommendations");
  });

  it("refuses anything outside the audited surface before the IPC boundary", async () => {
    for (const command of [
      "delete_receipt",
      "execute_sql",
      "egress_receipts; DROP TABLE egress_log",
      "",
    ]) {
      await expect(readCommand(command)).rejects.toThrow(
        "is not an audited read command",
      );
    }
    expect(invoke).not.toHaveBeenCalled();
  });

  it("recognizes every audited read command", () => {
    expect(isReadCommand("daily_recommendations")).toBe(true);
    expect(isReadCommand("privacy_status")).toBe(true);
    expect(isReadCommand("egress_receipts")).toBe(false);
    expect(isReadCommand("write_receipt")).toBe(false);
  });

  it("exposes a typed fetcher for the daily view", async () => {
    invoke.mockResolvedValueOnce([{ id: "rec-1" }]);

    await expect(fetchDailyRecommendations()).resolves.toEqual([
      { id: "rec-1" },
    ]);
  });
});

describe("inputCommand", () => {
  it("recognizes the receipts cursor as an input, not a read", () => {
    // The receipts walk carries a pagination payload, so it lives on the
    // audited input side even though it only ever reads the store.
    expect(isInputCommand("egress_receipts")).toBe(true);
    expect(isReadCommand("egress_receipts")).toBe(false);
  });

  it("invokes an audited input command with its named payload", async () => {
    invoke.mockResolvedValueOnce([]);
    const cursor = {
      beforeCreatedAt: "2026-09-25T10:00:00.000Z",
      beforeId: "eg-1",
    };

    await expect(fetchEgressReceipts(cursor)).resolves.toEqual([]);
    expect(invoke).toHaveBeenCalledWith("egress_receipts", { input: cursor });
  });

  it("refuses anything outside the audited surface before the IPC boundary", async () => {
    for (const command of [
      "write_receipt",
      "execute_sql",
      "egress_receipts; DROP TABLE egress_log",
      "",
    ]) {
      await expect(inputCommand(command, {})).rejects.toThrow(
        "is not an audited input command",
      );
    }
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("errorMessage", () => {
  // Tauri serializes AppError to a plain display string.
  it("passes backend display strings through untouched", () => {
    expect(errorMessage("store error: no such table")).toBe(
      "store error: no such table",
    );
  });

  it("normalizes Error objects and anything else to text", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage(42)).toBe("42");
    expect(errorMessage(undefined)).toBe("undefined");
  });
});
