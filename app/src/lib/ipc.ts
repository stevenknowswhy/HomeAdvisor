//! The webview's one door to the Rust core: the audited read commands, and
//! nothing else.
//!
//! The backend's surface audit (`app/src-tauri/src/audit.rs`) is the
//! authority on what may be invoked: exactly three commands, each taking
//! only managed state — no SQL, no paths, no network targets from the
//! webview. This module mirrors that surface on the frontend side and keeps
//! it single-sourced: every IPC call in the app goes through
//! [`readCommand`], which refuses anything outside [`READ_COMMANDS`]. The
//! frontend surface test pins this file as the only `invoke` site, so a
//! write path cannot reach the core without failing the audit twice.

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

// ───────────────────────── the audited surface ────────────────────

/** The commands the backend registers and the surface audit permits —
 *  single-sourced here to mirror `app_commands!` in `commands.rs`. */
export const READ_COMMANDS = [
  "daily_recommendations",
  "egress_receipts",
  "privacy_status",
] as const;

export type ReadCommand = (typeof READ_COMMANDS)[number];

export function isReadCommand(name: string): name is ReadCommand {
  return (READ_COMMANDS as readonly string[]).includes(name);
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

/** The daily view's source: recommendations currently marked served,
 *  newest first, each with its evidence links. */
export function fetchDailyRecommendations(): Promise<RecommendationView[]> {
  return readCommand<RecommendationView[]>("daily_recommendations");
}

/** The privacy receipts screen's source: every egress-log row, newest
 *  first, read-only. */
export function fetchEgressReceipts(): Promise<ReceiptView[]> {
  return readCommand<ReceiptView[]>("egress_receipts");
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
