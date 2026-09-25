// Preview-only stand-in for the Tauri IPC boundary. The real boundary is
// `@tauri-apps/api/core`'s `invoke` over Tauri IPC; in a plain browser there
// is no backend, so `vite.preview.config.ts` aliases this module in its place
// to dogfood the views with fixture data. It is never imported by shipped
// code — the production bundle uses the real `invoke`.
//
// The scenario comes from `?scenario=`: `populated` (default), `empty`, or
// `error` — mirroring the three states the component tests cover.

import type { RecommendationView, ReceiptView } from "../ipc";

const scenario = new URLSearchParams(window.location.search).get(
  "scenario",
) as "populated" | "empty" | "error" | null;

const recommendations: RecommendationView[] = [
  {
    id: "rec-1",
    goalId: "goal-emergency-fund",
    category: "wealth",
    title: "Automate the emergency-fund transfer",
    explanation:
      "A standing transfer the day after payday removes the willpower step — the fund grows without a monthly decision.",
    recommendationType: "task",
    effortEstimate: "10 minutes",
    expectedBenefit: "one less monthly decision",
    confidence: 0.72,
    status: "served",
    whyMe: "Your emergency fund is the goal you ranked highest this quarter.",
    whyNow: "Payday is Friday; set the transfer before the balance lands.",
    createdAt: "2026-09-25T08:00:00.000Z",
    expiresAt: null,
    evidence: [
      {
        id: "ev-1",
        sourceType: "study",
        sourceUrl: "https://example.org/study",
        sourceTitle: "A source",
        publicationDate: "2026-01-01",
      },
    ],
  },
  {
    id: "rec-2",
    goalId: null,
    category: "health",
    title: "Book the swim assessment for the kids",
    explanation:
      "Two of your goals name water confidence; the municipal pool runs free assessments this month.",
    recommendationType: "appointment",
    effortEstimate: null,
    expectedBenefit: null,
    confidence: 0.55,
    status: "served",
    whyMe: null,
    whyNow: "Assessment slots fill by the end of the month.",
    createdAt: "2026-09-25T08:00:00.000Z",
    expiresAt: "2026-09-30T00:00:00.000Z",
    evidence: [],
  },
];

const receipts: ReceiptView[] = [
  {
    id: "eg-1",
    purpose: "domain research — wealth",
    decision: "ALLOW",
    payloadHash: "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e",
    createdAt: "2026-09-25 09:14:02Z",
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: ["child.name"],
      generalized: ["household.income"],
    }),
    layaScanJson: JSON.stringify({
      first: {
        per_class: [
          ["FullName", 0.02],
          ["UniqueCombination", 0.31],
        ],
        confidence: 0.92,
      },
    }),
    layaModelVersion: "laya-mini-2026-06",
    reason: null,
  },
  {
    id: "eg-2",
    purpose: "domain research — education",
    decision: "QUARANTINE",
    payloadHash: "4c8b1d0f7a3e9c5b1f8d2a6e0c4b8f2d6a0c4b8f",
    createdAt: "2026-09-25 11:40:17Z",
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: [],
      generalized: ["person.age"],
    }),
    layaScanJson: JSON.stringify({
      first: {
        per_class: [["UniqueCombination", 0.84]],
        confidence: 0.77,
      },
    }),
    layaModelVersion: "laya-mini-2026-06",
    reason: "leak class above threshold: unique combination",
  },
  {
    id: "eg-3",
    purpose: "domain research — health",
    decision: "BLOCK",
    payloadHash: "2e6a0c4b8f2d6a0e4c8b1d0f7a3e9c5b1f8d2a6e",
    createdAt: "2026-09-25 18:22:55Z",
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: [],
      generalized: [],
    }),
    layaScanJson: JSON.stringify({
      first: { Unavailable: ["connection refused"] },
    }),
    layaModelVersion: null,
    reason: "sidecar unavailable — fail closed",
  },
];

export async function invoke(command: string): Promise<unknown> {
  if (scenario === "error") {
    throw "store error: database key not supplied";
  }
  switch (command) {
    case "daily_recommendations":
      return scenario === "empty" ? [] : recommendations;
    case "egress_receipts":
      return scenario === "empty" ? [] : receipts;
    case "privacy_status":
      return { sidecar: "running", store: "open" };
    default:
      throw `unknown command: ${command}`;
  }
}
