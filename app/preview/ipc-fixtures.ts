/**
 * In-browser fixture IPC for the UI preview (`preview.html`). The Tauri
 * v2 JS layer routes every `invoke` through `window.__TAURI_INTERNALS__`,
 * so installing a fixture provider there lets a plain browser render the
 * real components against demo data. Nothing here is reachable from the
 * production entry: `main.ts` mounts without this module and talks to
 * the Rust core only.
 */

interface FixtureMember {
  id: string;
  household_id: string;
  role: string;
  display_name: string | null;
  age_band_id: string;
  school_stage: string | null;
  is_child: boolean;
}

interface FixtureGoal {
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

interface FixtureHousehold {
  id: string;
  timezone: string;
  locale: string | null;
  region_class: string;
  income_band_id: string | null;
}

interface InputBag {
  input?: Record<string, unknown>;
}

const rows: {
  household: FixtureHousehold | null;
  members: FixtureMember[];
  goals: FixtureGoal[];
  seq: number;
} = {
  household: null,
  members: [],
  goals: [],
  seq: 0,
};

function nextId(prefix: string): string {
  rows.seq += 1;
  return `${prefix}_preview_${rows.seq}`;
}

function str(input: Record<string, unknown>, key: string): string | null {
  const value = input[key];
  return typeof value === "string" && value.length > 0 ? value : null;
}

const handlers: Record<string, (args: InputBag) => unknown> = {
  get_household: () => rows.household,

  // The PR #11 read surface, in fixture form: the composed preview renders
  // the real DailyView and ReceiptsScreen against these.
  daily_recommendations: () => [
    {
      id: "rec_preview_1",
      goalId: rows.goals[0]?.id ?? null,
      category: "education",
      title: "Schedule the school-year checkup",
      explanation:
        "The goal you ranked first has a target date inside the next quarter; slots fill weeks ahead.",
      recommendationType: "action",
      effortEstimate: "10 minutes",
      expectedBenefit: "Beat the seasonal waitlist",
      confidence: 0.8,
      status: "served",
      whyMe: null,
      whyNow: null,
      createdAt: new Date().toISOString(),
      expiresAt: null,
      evidence: [],
    },
  ],

  // The fixture store never egresses anything - an honest empty state.
  egress_receipts: () => [],

  update_household: ({ input = {} }) => {
    if (!rows.household) throw new Error("no household yet");
    if (typeof input.timezone === "string" && input.timezone) {
      rows.household.timezone = input.timezone;
    }
    if (typeof input.locale === "string") rows.household.locale = input.locale || null;
    if (typeof input.region_class === "string" && input.region_class) {
      rows.household.region_class = input.region_class;
    }
    if (typeof input.income_band_id === "string" && input.income_band_id) {
      rows.household.income_band_id = input.income_band_id;
    }
    return rows.household;
  },

  create_household: ({ input = {} }) => {
    const regionClass = str(input, "region_class");
    const incomeBand = str(input, "income_band_id");
    if (!regionClass || !incomeBand) {
      throw new Error("region class and income band are required");
    }
    rows.household = {
      id: nextId("hh"),
      timezone: str(input, "timezone") ?? "UTC",
      locale: str(input, "locale"),
      region_class: regionClass,
      income_band_id: incomeBand,
    };
    return rows.household;
  },

  add_member: ({ input = {} }) => {
    if (!rows.household) throw new Error("no household yet");
    const role = str(input, "role") ?? "adult";
    const member: FixtureMember = {
      id: nextId("mem"),
      household_id: rows.household.id,
      role,
      display_name: str(input, "display_name"),
      age_band_id: str(input, "age_band_id") ?? "age_25_34",
      school_stage: str(input, "school_stage"),
      is_child: role === "child",
    };
    rows.members.push(member);
    return member;
  },

  list_members: () => rows.members,

  create_goal: ({ input = {} }) => {
    if (!rows.household) throw new Error("no household yet");
    const goal: FixtureGoal = {
      id: nextId("goal"),
      household_id: rows.household.id,
      person_id: str(input, "person_id"),
      title: str(input, "title") ?? "",
      detail: str(input, "detail"),
      domain: str(input, "domain") ?? "education",
      importance: typeof input.importance === "number" ? input.importance : 5,
      timeframe_start: str(input, "timeframe_start"),
      target_date: str(input, "target_date"),
      status: "active",
      progress: 0,
    };
    rows.goals.push(goal);
    return goal;
  },

  update_goal: ({ input = {} }) => {
    const goalId = str(input, "goal_id");
    const goal = rows.goals.find((row) => row.id === goalId);
    if (!goal) throw new Error("goal not found");
    if (typeof input.title === "string") goal.title = input.title;
    if (typeof input.detail === "string") goal.detail = input.detail;
    if (typeof input.domain === "string") goal.domain = input.domain;
    if (typeof input.importance === "number") goal.importance = input.importance;
    if (typeof input.timeframe_start === "string") {
      goal.timeframe_start = input.timeframe_start;
    }
    if (typeof input.target_date === "string") goal.target_date = input.target_date;
    if (typeof input.status === "string") goal.status = input.status;
    if (typeof input.progress === "number") goal.progress = input.progress;
    return goal;
  },

  list_goals: () =>
    [...rows.goals].sort((a, b) => b.importance - a.importance),

  delete_goal: ({ input = {} }) => {
    const goalId = str(input, "goal_id");
    rows.goals = rows.goals.filter((row) => row.id !== goalId);
  },
};

export function installPreviewIpc(): void {
  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: (cmd: string, args: InputBag) => {
      const handler = handlers[cmd];
      if (!handler) {
        // Fail closed on unknown commands, like the audited surface does.
        return Promise.reject(new Error(`unknown command: ${cmd}`));
      }
      return Promise.resolve(handler(args));
    },
  };
}
