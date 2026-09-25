//! The webview's one door to the Rust core: the audited read commands and
//! the audited input commands, and nothing else.
//!
//! The backend's surface audit (`app/src-tauri/src/audit.rs`) is the
//! authority on what may be invoked: two managed-state-only read commands
//! plus input commands that each take exactly one named `*Input` payload
//! (the onboarding writes, and the receipts screen's pagination cursor) —
//! no SQL, no paths, no network targets from the webview. This
//! module mirrors that surface on the frontend side and keeps it
//! single-sourced: every IPC call in the app goes through [`readCommand`]
//! or [`inputCommand`], which refuse anything outside their lists. The
//! frontend surface test pins this file as the only `invoke` site, so an
//! unaudited call cannot reach the core without failing the audit twice.

import { invoke } from "@tauri-apps/api/core";

// ───────────────────────────── views ──────────────────────────────
//
// TypeScript mirrors of the serde views in `app/src-tauri/src/commands.rs`
// (serde camelCase). The Rust structs are authoritative; the surface test
// keeps this file the only place that speaks IPC so drift is reviewable.

/** One evidence link behind a recommendation (`EvidenceView`). */
export interface EvidenceView {
  id: string;
  sourceType: string;
  sourceUrl: string | null;
  sourceTitle: string | null;
  publicationDate: string | null;
}

/** One served recommendation for the daily view (`RecommendationView`). */
export interface RecommendationView {
  id: string;
  goalId: string | null;
  category: string;
  /** Family-facing free text (`title_local` in the store). */
  title: string;
  /** Family-facing free text (`explanation_local` in the store). */
  explanation: string;
  recommendationType: string;
  effortEstimate: string | null;
  expectedBenefit: string | null;
  confidence: number | null;
  status: string;
  /** Family-facing free text (`why_me_local` in the store). */
  whyMe: string | null;
  /** Family-facing free text (`why_now_local` in the store). */
  whyNow: string | null;
  createdAt: string;
  expiresAt: string | null;
  evidence: EvidenceView[];
}

/** One row of the append-only egress log (`ReceiptView`). */
export interface ReceiptView {
  id: string;
  purpose: string;
  /** The exact generalized payload the decision covered. */
  payloadJson: string;
  payloadHash: string;
  transformationVersion: string;
  /** Layer 1's verdict — `Layer1Verdict` JSON or a plain recorded string. */
  layer1Verdict: string;
  /** The semantic scan trail — `ScanTrail` JSON when a scan ran. */
  layaScanJson: string | null;
  layaModelVersion: string | null;
  decision: string;
  reason: string | null;
  createdAt: string;
}

/** The webview-side page size of the receipts walk. Keep in lockstep with
 *  `RECEIPTS_PAGE_SIZE` in `commands.rs` — a full page is the signal that
 *  more rows may exist, so the two sides must agree or the load-more
 *  control shows and hides one call out of phase. */
export const RECEIPTS_PAGE_SIZE = 200;

/** The receipts pagination cursor (`EgressReceiptsInput` in commands.rs,
 *  serde camelCase): the `created_at` and `id` of the last receipt of the
 *  previous page, or both `null` for the first page. The pair travels
 *  together — the backend rejects half a cursor. */
export interface EgressReceiptsInput {
  beforeCreatedAt: string | null;
  beforeId: string | null;
}

// ───────────────────────── the audited surface ────────────────────

/** The commands the backend registers and the surface audit permits —
 *  single-sourced here to mirror `app_commands!` in `commands.rs`. */
export const READ_COMMANDS = [
  "daily_recommendations",
  "privacy_status",
  "get_household",
] as const;

export type ReadCommand = (typeof READ_COMMANDS)[number];

export function isReadCommand(name: string): name is ReadCommand {
  return (READ_COMMANDS as readonly string[]).includes(name);
}

/** The commands that each take exactly one named `*Input` payload — the
 *  audited input side (onboarding writes, and the receipts screen's
 *  pagination cursor). Single-sourced to mirror `app_commands!` in
 *  `commands.rs`; the backend signature audit permits these because their
 *  only webview-controlled parameter is a reviewed `*Input` struct. */
export const INPUT_COMMANDS = [
  "egress_receipts",
  "create_household",
  "update_household",
  "add_member",
  "list_members",
  "create_goal",
  "update_goal",
  "delete_goal",
  "list_goals",
] as const;

export type InputCommand = (typeof INPUT_COMMANDS)[number];

export function isInputCommand(name: string): name is InputCommand {
  return (INPUT_COMMANDS as readonly string[]).includes(name);
}

/** Invoke one audited read command. Anything else — a write, a delete, a
 *  made-up name — is refused before the IPC boundary, fail-closed. */
export async function readCommand<T>(command: string): Promise<T> {
  if (!isReadCommand(command)) {
    throw new Error(
      `"${command}" is not an audited read command — the webview may only read`,
    );
  }
  return invoke<T>(command);
}

/** Invoke one audited input command with its named payload. Anything
 *  outside the list — a made-up name, a bare primitive, a second
 *  parameter — is refused before the IPC boundary, fail-closed. */
export async function inputCommand<T>(
  command: string,
  input: unknown,
): Promise<T> {
  if (!isInputCommand(command)) {
    throw new Error(
      `"${command}" is not an audited input command — the webview may only send reviewed payloads`,
    );
  }
  return invoke<T>(command, { input });
}

/** The daily view's source: recommendations currently marked served,
 *  newest first, each with its evidence links. */
export function fetchDailyRecommendations(): Promise<RecommendationView[]> {
  return readCommand<RecommendationView[]>("daily_recommendations");
}

/** The privacy receipts screen's source: one page of the egress log,
 *  newest first, read-only. The first page passes no cursor; every next
 *  page passes the `createdAt`/`id` of the last receipt shown. */
export function fetchEgressReceipts(
  input: EgressReceiptsInput,
): Promise<ReceiptView[]> {
  return inputCommand<ReceiptView[]>("egress_receipts", input);
}

// ───────────────────────────── errors ─────────────────────────────

/** The backend serializes `AppError` to a plain display string, so a failed
 *  invoke rejects with a string. Normalize every shape to display text —
 *  the views show one clear message, never a stack trace. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
