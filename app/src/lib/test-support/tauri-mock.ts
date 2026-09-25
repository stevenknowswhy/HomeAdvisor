// Preview-only stand-in for the Tauri IPC boundary. The real boundary is
// `@tauri-apps/api/core`'s `invoke` over Tauri IPC; in a plain browser there
// is no backend, so `vite.preview.config.ts` aliases this module in its place
// to dogfood the views with fixture data. It is never imported by shipped
// code — the production bundle uses the real `invoke`.
//
// The scenario comes from `?scenario=`: `populated` (default), `empty`, or
// `error` — mirroring the three states the component tests cover.

import { RECEIPTS_PAGE_SIZE } from "../ipc";
import type {
  EgressReceiptsInput,
  RecommendationView,
  ReceiptView,
} from "../ipc";

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
    payloadJson: JSON.stringify({
      url: "https://example.com/research/wealth",
      query: "college savings strategies",
    }),
    decision: "ALLOW",
    payloadHash: "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e",
    transformationVersion: "layer1-2026-06",
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
    payloadJson: JSON.stringify({
      url: "https://example.com/research/education",
      query: "swim lessons for children",
    }),
    decision: "QUARANTINE",
    payloadHash: "4c8b1d0f7a3e9c5b1f8d2a6e0c4b8f2d6a0c4b8f",
    transformationVersion: "layer1-2026-06",
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
    payloadJson: JSON.stringify({
      url: "https://example.com/research/health",
      query: "toddler sleep schedule",
    }),
    decision: "BLOCK",
    payloadHash: "2e6a0c4b8f2d6a0e4c8b1d0f7a3e9c5b1f8d2a6e",
    transformationVersion: "layer1-2026-06",
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

// Older fixture receipts for the pagination walk: enough rows below the
// hand-written receipts that the load-more control has pages to fetch
// (two full preview pages plus a terminal one). Timestamps descend
// strictly, mirroring the append-only log the real command pages over.
const OLDER_RECEIPT_COUNT = 420;

const olderReceipts: ReceiptView[] = Array.from(
  { length: OLDER_RECEIPT_COUNT },
  (_, n) => ({
    id: `eg-gen-${String(n).padStart(4, "0")}`,
    purpose: "domain research — wealth",
    payloadJson: JSON.stringify({
      url: "https://example.com/research/wealth",
      query: "college savings strategies",
    }),
    payloadHash: `gen${String(n).padStart(4, "0")}4b6a8c0e2d4b6a8c`,
    transformationVersion: "layer1-2026-06",
    createdAt: `2026-09-25 09:${String(13 - Math.floor(n / 60)).padStart(2, "0")}:${String(59 - (n % 60)).padStart(2, "0")}Z`,
    layer1Verdict: JSON.stringify({
      status: "passed",
      removed: ["child.name"],
      generalized: ["household.income"],
    }),
    layaScanJson: JSON.stringify({
      first: { per_class: [["FullName", 0.02]], confidence: 0.92 },
    }),
    layaModelVersion: "laya-mini-2026-06",
    decision: n % 3 === 0 ? "BLOCK" : n % 3 === 1 ? "QUARANTINE" : "ALLOW",
    reason:
      n % 3 === 0 ? "leak class above threshold: unique combination" : null,
  }),
);

function pageReceipts(input: unknown): ReceiptView[] {
  const cursor = (input ?? {}) as Partial<EgressReceiptsInput>;
  const all = [...receipts, ...olderReceipts];
  if (cursor.beforeCreatedAt == null || cursor.beforeId == null) {
    return all.slice(0, RECEIPTS_PAGE_SIZE);
  }
  const cursorIndex = all.findIndex(
    (receipt) =>
      receipt.createdAt === cursor.beforeCreatedAt &&
      receipt.id === cursor.beforeId,
  );
  if (cursorIndex === -1) {
    throw "unknown receipt cursor — the page cannot be found in this log";
  }
  return all.slice(cursorIndex + 1, cursorIndex + 1 + RECEIPTS_PAGE_SIZE);
}

// The onboarding and household-management commands, kept in-memory: the
// preview starts with no household and grows one as the operator walks the
// forms, so the receipts and daily fixtures read like a real store.
interface MockHousehold {
  id: string;
  timezone: string;
  locale: string | null;
  region_class: string;
  income_band_id: string | null;
}

interface MockMember {
  id: string;
  household_id: string;
  role: string;
  display_name: string | null;
  age_band_id: string;
  school_stage: string | null;
  is_child: boolean;
}

interface MockGoal {
  id: string;
  household_id: string;
  person_id: string | null;
  title: string;
  detail: string | null;
  domain: string;
  importance: number;
  timeframe_start: string | null;
  target_date: string | null;
  status: string;
  progress: number;
}

let household: MockHousehold | null = null;
let members: MockMember[] = [];
let goals: MockGoal[] = [];

function inputOf<T>(args?: { input?: unknown }): T {
  return (args?.input ?? {}) as T;
}

export async function invoke(
  command: string,
  args?: { input?: unknown },
): Promise<unknown> {
  if (scenario === "error") {
    throw "store error: database key not supplied";
  }
  switch (command) {
    case "daily_recommendations":
      return scenario === "empty" ? [] : recommendations;
    case "egress_receipts":
      return scenario === "empty" ? [] : pageReceipts(args?.input);
    case "privacy_status":
      return { sidecar: "running", store: "open" };
    case "get_household":
      return household;
    case "create_household": {
      const input = inputOf<{
        timezone: string;
        locale: string | null;
        region_class: string;
        income_band_id: string;
      }>(args);
      household = {
        id: "hh-preview",
        timezone: input.timezone,
        locale: input.locale,
        region_class: input.region_class,
        income_band_id: input.income_band_id,
      };
      return household;
    }
    case "update_household": {
      const input = inputOf<{
        household_id: string;
        timezone: string;
        locale: string | null;
        region_class: string;
        income_band_id: string | null;
      }>(args);
      household = {
        id: input.household_id,
        timezone: input.timezone,
        locale: input.locale,
        region_class: input.region_class,
        income_band_id: input.income_band_id,
      };
      return household;
    }
    case "add_member": {
      const input = inputOf<{
        household_id: string;
        role: string;
        display_name: string | null;
        age_band_id: string;
        school_stage: string | null;
      }>(args);
      const member: MockMember = {
        id: `member-${members.length + 1}`,
        household_id: input.household_id,
        role: input.role,
        display_name: input.display_name,
        age_band_id: input.age_band_id,
        school_stage: input.school_stage,
        is_child: input.role === "child",
      };
      members.push(member);
      return member;
    }
    case "list_members": {
      const input = inputOf<{ household_id: string }>(args);
      return members.filter((m) => m.household_id === input.household_id);
    }
    case "create_goal": {
      const input = inputOf<{
        household_id: string;
        person_id: string | null;
        title: string;
        detail: string | null;
        domain: string;
        importance: number;
        timeframe_start: string | null;
        target_date: string | null;
      }>(args);
      const goal: MockGoal = {
        id: `goal-${goals.length + 1}`,
        household_id: input.household_id,
        person_id: input.person_id,
        title: input.title,
        detail: input.detail,
        domain: input.domain,
        importance: input.importance,
        timeframe_start: input.timeframe_start,
        target_date: input.target_date,
        status: "active",
        progress: 0,
      };
      goals.push(goal);
      return goal;
    }
    case "update_goal": {
      const input = inputOf<{
        goal_id: string;
        title: string;
        detail: string | null;
        domain: string;
        importance: number;
        timeframe_start: string | null;
        target_date: string | null;
        status: string;
        progress: number;
      }>(args);
      const goal = goals.find((g) => g.id === input.goal_id);
      if (goal === undefined) throw `unknown goal: ${input.goal_id}`;
      goal.title = input.title;
      goal.detail = input.detail;
      goal.domain = input.domain;
      goal.importance = input.importance;
      goal.timeframe_start = input.timeframe_start;
      goal.target_date = input.target_date;
      goal.status = input.status;
      goal.progress = input.progress;
      return goal;
    }
    case "delete_goal": {
      const input = inputOf<{ goal_id: string }>(args);
      goals = goals.filter((g) => g.id !== input.goal_id);
      return null;
    }
    case "list_goals": {
      const input = inputOf<{ household_id: string }>(args);
      return goals.filter((g) => g.household_id === input.household_id);
    }
    default:
      throw `unknown command: ${command}`;
  }
}
