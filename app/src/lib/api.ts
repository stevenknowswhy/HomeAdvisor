/**
 * Typed wrappers over the Tauri IPC commands. Every write payload is a
 * named `*Input` struct — the one webview-controlled shape the surface
 * audit (`app/src-tauri/src/audit.rs`) allows, mirrored here field for
 * field. Sensitive attributes travel as band ids only; the forms never
 * collect a raw age, income, or address.
 */
import { errorMessage, inputCommand, readCommand } from "./ipc";

export { errorMessage };

export interface HouseholdView {
  id: string;
  timezone: string;
  locale: string | null;
  region_class: string;
  income_band_id: string | null;
}

export interface MemberView {
  id: string;
  household_id: string;
  role: string;
  display_name: string | null;
  age_band_id: string;
  school_stage: string | null;
  is_child: boolean;
}

export interface GoalView {
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

export interface CreateHouseholdInput {
  timezone: string;
  locale: string | null;
  region_class: string;
  income_band_id: string;
}

export interface UpdateHouseholdInput {
  household_id: string;
  timezone: string;
  locale: string | null;
  region_class: string;
  income_band_id: string;
}

export interface AddMemberInput {
  household_id: string;
  role: string;
  display_name: string | null;
  age_band_id: string;
  school_stage: string | null;
}

export interface ListMembersInput {
  household_id: string;
}

/** Mirrors the Rust `ListGoalsInput` — same shape today, a separate type
 *  so the two surfaces can drift deliberately under review. */
export interface ListGoalsInput {
  household_id: string;
}

export interface CreateGoalInput {
  household_id: string;
  person_id: string | null;
  title: string;
  detail: string | null;
  domain: string;
  importance: number;
  timeframe_start: string | null;
  target_date: string | null;
}

export interface UpdateGoalInput {
  goal_id: string;
  title: string;
  detail: string | null;
  domain: string;
  importance: number;
  timeframe_start: string | null;
  target_date: string | null;
  status: string;
  progress: number;
}

export interface DeleteGoalInput {
  goal_id: string;
}

export async function getHousehold(): Promise<HouseholdView | null> {
  return readCommand<HouseholdView | null>("get_household");
}

export async function createHousehold(
  input: CreateHouseholdInput,
): Promise<HouseholdView> {
  return inputCommand<HouseholdView>("create_household", input);
}

export async function updateHousehold(
  input: UpdateHouseholdInput,
): Promise<HouseholdView> {
  return inputCommand<HouseholdView>("update_household", input);
}

export async function addMember(input: AddMemberInput): Promise<MemberView> {
  return inputCommand<MemberView>("add_member", input);
}

export async function listMembers(
  input: ListMembersInput,
): Promise<MemberView[]> {
  return inputCommand<MemberView[]>("list_members", input);
}

export async function createGoal(input: CreateGoalInput): Promise<GoalView> {
  return inputCommand<GoalView>("create_goal", input);
}

export async function updateGoal(input: UpdateGoalInput): Promise<GoalView> {
  return inputCommand<GoalView>("update_goal", input);
}

export async function deleteGoal(input: DeleteGoalInput): Promise<void> {
  return inputCommand<void>("delete_goal", input);
}

export async function listGoals(input: ListGoalsInput): Promise<GoalView[]> {
  return inputCommand<GoalView[]>("list_goals", input);
}
