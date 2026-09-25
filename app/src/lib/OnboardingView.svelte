<script lang="ts">
  /**
   * The onboarding surface: create the household (band-typed profile),
   * then manage members and goals. Sensitive attributes — income, region,
   * ages — are picked from the v1 bands, never typed as raw values; only
   * the timezone, locale, member names, and goal text are free fields, and
   * the store quarantines those (`*_local`) from every outbound path.
   */
  import { onMount } from "svelte";

  import * as api from "./api";
  import { errorMessage } from "./api";
  import {
    ALL_AGE_BANDS,
    INCOME_BANDS,
    REGION_CLASSES,
    SCHOOL_STAGES,
    bandLabel,
  } from "./bands";
  import BandPicker from "./BandPicker.svelte";
  import GoalForm from "./GoalForm.svelte";
  import MemberForm from "./MemberForm.svelte";

  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let household = $state<api.HouseholdView | null>(null);
  let members = $state<api.MemberView[]>([]);
  let goals = $state<api.GoalView[]>([]);

  // Household creation form state.
  let timezone = $state("");
  let locale = $state("");
  let regionClass = $state<string | null>(null);
  let incomeBand = $state<string | null>(null);
  let creating = $state(false);
  let createError = $state<string | null>(null);

  // Goal editing state: null hides the create form while an edit is open.
  let editingGoal = $state<api.GoalView | null>(null);
  let goalBusyId = $state<string | null>(null);

  onMount(refresh);

  async function refresh(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      household = await api.getHousehold();
      if (household) {
        members = await api.listMembers({ household_id: household.id });
        goals = await api.listGoals({ household_id: household.id });
      }
    } catch (e) {
      loadError = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function submitHousehold(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!regionClass) {
      createError = "Pick the region class that fits your home.";
      return;
    }
    if (!incomeBand) {
      createError = "Pick an income band — the exact number never enters the app.";
      return;
    }
    creating = true;
    createError = null;
    try {
      const created = await api.createHousehold({
        timezone: timezone.trim() || "UTC",
        locale: locale.trim() || null,
        region_class: regionClass,
        income_band_id: incomeBand,
      });
      household = created;
      members = await api.listMembers({ household_id: created.id });
      goals = await api.listGoals({ household_id: created.id });
    } catch (e) {
      createError = errorMessage(e);
    } finally {
      creating = false;
    }
  }

  async function memberAdded(member: api.MemberView): Promise<void> {
    members = [...members, member];
  }

  async function goalSaved(goal: api.GoalView): Promise<void> {
    editingGoal = null;
    const index = goals.findIndex((existing) => existing.id === goal.id);
    const next =
      index === -1
        ? [...goals, goal]
        : goals.map((existing) => (existing.id === goal.id ? goal : existing));
    goals = next.sort(
      (a, b) => b.importance - a.importance || a.title.localeCompare(b.title),
    );
  }

  async function deleteGoal(goal: api.GoalView): Promise<void> {
    goalBusyId = goal.id;
    try {
      await api.deleteGoal({ goal_id: goal.id });
      goals = goals.filter((existing) => existing.id !== goal.id);
    } catch (e) {
      loadError = errorMessage(e);
    } finally {
      goalBusyId = null;
    }
  }
</script>

<section aria-labelledby="onboarding-heading">
  <h2 id="onboarding-heading">Your household</h2>

  {#if loading}
    <p class="onboarding-loading" aria-live="polite">Loading your household…</p>
  {:else if loadError}
    <p class="onboarding-error" role="alert">
      <span>{loadError}</span>
      <button type="button" onclick={refresh}>Try again</button>
    </p>
  {:else if !household}
    <p class="section-hint">
      Three steps: the household profile, the people in it, and the goals
      that matter. Sensitive facts — income, region, ages — are picked from
      bands, never typed in, so the exact values cannot be stored or sent.
    </p>

    <form class="household-form" onsubmit={submitHousehold}>
      <label class="field">
        <span>Time zone</span>
        <input
          type="text"
          bind:value={timezone}
          placeholder="e.g. America/Chicago"
          aria-label="Time zone"
        />
      </label>
      <label class="field">
        <span>Language (optional)</span>
        <input
          type="text"
          bind:value={locale}
          placeholder="e.g. en-US"
          aria-label="Locale"
        />
      </label>

      <BandPicker
        id="household-region-class"
        label="Region class"
        options={REGION_CLASSES}
        value={regionClass}
        onSelect={(value) => (regionClass = value)}
      />
      <BandPicker
        id="household-income-band"
        label="Household income band"
        options={INCOME_BANDS}
        value={incomeBand}
        onSelect={(value) => (incomeBand = value)}
      />

      {#if createError}
        <p class="onboarding-error" role="alert">{createError}</p>
      {/if}

      <button type="submit" disabled={creating}>
        {creating ? "Creating…" : "Create household"}
      </button>
    </form>
  {:else}
    <dl class="household-profile">
      <div>
        <dt>Region</dt>
        <dd>{bandLabel(REGION_CLASSES, household.region_class)}</dd>
      </div>
      <div>
        <dt>Income band</dt>
        <dd>{bandLabel(INCOME_BANDS, household.income_band_id)}</dd>
      </div>
      <div>
        <dt>Time zone</dt>
        <dd>{household.timezone}</dd>
      </div>
      {#if household.locale}
        <div>
          <dt>Language</dt>
          <dd>{household.locale}</dd>
        </div>
      {/if}
    </dl>

    <section aria-labelledby="members-heading">
      <h3 id="members-heading">Members</h3>
      {#if members.length === 0}
        <p class="section-hint">No members yet — add the first one below.</p>
      {:else}
        <ul class="member-list">
          {#each members as member (member.id)}
            <li>
              <strong>{member.display_name ?? "Unnamed member"}</strong>
              <span class="member-meta">
                {member.role} · age band {bandLabel(ALL_AGE_BANDS, member.age_band_id)}{#if member.school_stage}&nbsp;· {bandLabel(SCHOOL_STAGES, member.school_stage)}{/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      <MemberForm householdId={household.id} onAdded={memberAdded} />
    </section>

    <section aria-labelledby="goals-heading">
      <h3 id="goals-heading">Goals</h3>
      {#if goals.length === 0}
        <p class="section-hint">
          No goals yet — add the first one below. A few sharp goals beat a
          long list.
        </p>
      {:else}
        <ul class="goal-list">
          {#each goals as goal (goal.id)}
            <li>
              {#if editingGoal?.id === goal.id}
                <GoalForm
                  householdId={household.id}
                  members={members}
                  editing={goal}
                  onSaved={goalSaved}
                  onDismiss={() => (editingGoal = null)}
                />
              {:else}
                <div class="goal-row">
                  <div>
                    <strong>{goal.title}</strong>
                    <span class="goal-meta">
                      importance {goal.importance}/10
                      {#if goal.target_date}
                        · by {goal.target_date}
                      {/if}
                      · {goal.status}
                    </span>
                  </div>
                  <div class="goal-actions">
                    <button
                      type="button"
                      onclick={() => (editingGoal = goal)}
                      aria-label={`Edit goal ${goal.title}`}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      class="goal-delete"
                      disabled={goalBusyId === goal.id}
                      onclick={() => deleteGoal(goal)}
                      aria-label={`Delete goal ${goal.title}`}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if !editingGoal}
        <GoalForm
          householdId={household.id}
          members={members}
          onSaved={goalSaved}
        />
      {/if}
    </section>
  {/if}
</section>

<style>
  .onboarding-loading {
    color: var(--muted, #5b6675);
  }

  .onboarding-error {
    color: #b3261e;
    display: flex;
    gap: 0.6rem;
    align-items: center;
  }

  .section-hint {
    color: var(--muted, #5b6675);
  }

  .household-form {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-width: 30rem;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .field span {
    font-weight: 600;
  }

  input[type="text"] {
    padding: 0.5rem 0.7rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 8px;
    font: inherit;
  }

  button {
    align-self: flex-start;
    padding: 0.5rem 1.2rem;
    border: none;
    border-radius: 8px;
    background: #2f7d6d;
    color: #fff;
    font: inherit;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  .onboarding-error button,
  .goal-actions button {
    padding: 0.3rem 0.8rem;
    background: transparent;
    color: var(--fg, #1c2430);
    border: 1px solid var(--line, #e3e7ec);
  }

  .household-profile {
    display: flex;
    gap: 2rem;
    flex-wrap: wrap;
    margin: 1rem 0;
    padding: 0.8rem 1rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 10px;
  }

  .household-profile div {
    display: flex;
    flex-direction: column;
  }

  .household-profile dt {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #5b6675);
  }

  .household-profile dd {
    margin: 0;
    font-weight: 600;
  }

  h3 {
    margin-top: 1.6rem;
  }

  .member-list,
  .goal-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .member-list li,
  .goal-list li {
    padding: 0.6rem 0.9rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 10px;
  }

  .member-meta,
  .goal-meta {
    color: var(--muted, #5b6675);
    margin-left: 0.4rem;
    font-size: 0.9rem;
  }

  .goal-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
  }

  .goal-actions {
    display: flex;
    gap: 0.4rem;
  }

  .goal-delete {
    color: #b3261e !important;
    border-color: #b3261e !important;
  }
</style>
