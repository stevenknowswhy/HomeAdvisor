/**
 * The locked v1 band taxonomy (migration 002 in ha-store) and the enum
 * vocabularies the Rust core expects, mirrored as plain strings. Band
 * boundaries are contract, not UI state — editing them is a later
 * milestone behind the Generalizer trait.
 */
export interface BandOption {
  value: string;
  label: string;
}

/** Age bands for a child (adult bands continue below). */
export const CHILD_AGE_BANDS: BandOption[] = [
  { value: "age_0_2", label: "0-2" },
  { value: "age_3_5", label: "3-5" },
  { value: "age_6_9", label: "6-9" },
  { value: "age_10_12", label: "10-12" },
  { value: "age_13_15", label: "13-15" },
  { value: "age_16_17", label: "16-17" },
];

export const ADULT_AGE_BANDS: BandOption[] = [
  { value: "age_18_24", label: "18-24" },
  { value: "age_25_34", label: "25-34" },
  { value: "age_35_44", label: "35-44" },
  { value: "age_45_54", label: "45-54" },
  { value: "age_55_64", label: "55-64" },
  { value: "age_65_74", label: "65-74" },
  { value: "age_75_plus", label: "75+" },
];

/** Every age band, child first — for rendering stored band ids. */
export const ALL_AGE_BANDS: BandOption[] = [...CHILD_AGE_BANDS, ...ADULT_AGE_BANDS];

export const INCOME_BANDS: BandOption[] = [
  { value: "income_under_50k", label: "under-50k" },
  { value: "income_50k_75k", label: "50k-75k" },
  { value: "income_75k_100k", label: "75k-100k" },
  { value: "income_100k_150k", label: "100k-150k" },
  { value: "income_150k_200k", label: "150k-200k" },
  { value: "income_200k_plus", label: "200k+" },
];

export const REGION_CLASSES: BandOption[] = [
  { value: "urban_metro", label: "Urban metro" },
  { value: "suburban", label: "Suburban" },
  { value: "rural", label: "Rural" },
  { value: "small_town", label: "Small town" },
];

export const SCHOOL_STAGES: BandOption[] = [
  { value: "preschool", label: "Preschool" },
  { value: "elementary", label: "Elementary" },
  { value: "middle_school", label: "Middle school" },
  { value: "high_school", label: "High school" },
];

export const GOAL_DOMAINS: BandOption[] = [
  { value: "health", label: "Health" },
  { value: "wealth", label: "Wealth" },
  { value: "education", label: "Education" },
  { value: "career", label: "Career" },
  { value: "lifestyle", label: "Lifestyle" },
  { value: "connection", label: "Family happiness" },
];

export const GOAL_STATUSES: BandOption[] = [
  { value: "active", label: "Active" },
  { value: "paused", label: "Paused" },
  { value: "completed", label: "Completed" },
];

export function bandLabel(
  options: BandOption[],
  value: string | null | undefined,
): string {
  return options.find((option) => option.value === value)?.label ?? "—";
}
