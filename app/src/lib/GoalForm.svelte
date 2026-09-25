<script lang="ts">
  /**
   * Goal create/edit form. The goal is the center of gravity per the spec:
   * it carries an importance (1-10), a domain, an optional owner, and the
   * optional dates that make it deadline-aware. Title and detail are the
   * family's own words — quarantined `*_local` columns in the store.
   */
  import * as api from "./api";
  import { errorMessage } from "./api";
  import { GOAL_DOMAINS, GOAL_STATUSES } from "./bands";

  interface Props {
    householdId: string;
    /** Members who can own a goal; empty list = household-level only. */
    members: api.MemberView[];
    /** Present when editing an existing goal; absent for create. */
    editing?: api.GoalView | null;
    onSaved: (goal: api.GoalView) => void;
    onDismiss?: () => void;
  }

  let {
    householdId,
    members,
    editing = null,
    onSaved,
    onDismiss,
  }: Props = $props();

  // Seed once per mounted form. Each goal's edit form is rendered inline
  // under its own {#if}, so editing a different goal mounts a fresh form
  // and re-seeds — the non-reactive snapshot below is intentional.
  const seed = {
    title: editing?.title ?? "",
    detail: editing?.detail ?? "",
    personId: editing?.person_id ?? null,
    domain: editing?.domain ?? "education",
    importance: editing?.importance ?? 5,
    timeframeStart: editing?.timeframe_start ?? "",
    targetDate: editing?.target_date ?? "",
    status: editing?.status ?? "active",
    progress: editing?.progress ?? 0,
  };

  let title = $state(seed.title);
  let detail = $state(seed.detail);
  let personId = $state<string | null>(seed.personId);
  let domain = $state(seed.domain);
  let importance = $state(seed.importance);
  let timeframeStart = $state(seed.timeframeStart);
  let targetDate = $state(seed.targetDate);
  let status = $state(seed.status);
  let progress = $state(seed.progress);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!title.trim()) {
      error = "Give the goal a title.";
      return;
    }
    busy = true;
    error = null;
    try {
      if (editing) {
        const saved = await api.updateGoal({
          goal_id: editing.id,
          title: title.trim(),
          detail: detail.trim() || null,
          domain,
          importance,
          timeframe_start: timeframeStart || null,
          target_date: targetDate || null,
          status,
          progress,
        });
        onSaved(saved);
      } else {
        const saved = await api.createGoal({
          household_id: householdId,
          person_id: personId,
          title: title.trim(),
          detail: detail.trim() || null,
          domain,
          importance,
          timeframe_start: timeframeStart || null,
          target_date: targetDate || null,
        });
        title = "";
        detail = "";
        personId = null;
        importance = 5;
        timeframeStart = "";
        targetDate = "";
        onSaved(saved);
      }
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<form class="goal-form" onsubmit={submit}>
  <label class="field">
    <span>Goal</span>
    <input
      type="text"
      bind:value={title}
      placeholder="e.g. Reading fluency by spring"
      aria-label="Goal title"
    />
  </label>

  <label class="field">
    <span>Notes (optional)</span>
    <textarea bind:value={detail} rows="2" aria-label="Goal notes"></textarea>
    <small>For your eyes — notes stay on this device.</small>
  </label>

  <label class="field">
    <span>Who is it for?</span>
    <!-- Not bound: the empty option must map to null (household-level),
         not to the empty string the core would reject as an unknown id. -->
    <select
      value={personId ?? ""}
      aria-label="Goal owner"
      onchange={(event) => {
        personId = (event.target as HTMLSelectElement).value || null;
      }}
    >
      <option value="">The whole household</option>
      {#each members as member (member.id)}
        <option value={member.id}>
          {member.display_name ?? "Unnamed member"}
        </option>
      {/each}
    </select>
  </label>

  <div class="pair">
    <label class="field">
      <span>Domain</span>
      <select bind:value={domain} aria-label="Goal domain">
        {#each GOAL_DOMAINS as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </label>

    <label class="field">
      <span>Importance: {importance} / 10</span>
      <input
        type="range"
        min="1"
        max="10"
        step="1"
        bind:value={importance}
        aria-label="Goal importance"
      />
    </label>
  </div>

  <div class="pair">
    <label class="field">
      <span>Starting (optional)</span>
      <input
        type="date"
        bind:value={timeframeStart}
        aria-label="Goal start date"
      />
    </label>
    <label class="field">
      <span>Target date (optional)</span>
      <input type="date" bind:value={targetDate} aria-label="Goal target date" />
    </label>
  </div>

  {#if editing}
    <div class="pair">
      <label class="field">
        <span>Status</span>
        <select bind:value={status} aria-label="Goal status">
          {#each GOAL_STATUSES as option (option.value)}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      </label>
      <label class="field">
        <span>Progress: {Math.round(progress * 100)}%</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.1"
          bind:value={progress}
          aria-label="Goal progress"
        />
      </label>
    </div>
  {/if}

  {#if error}
    <p class="goal-form__error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <button type="submit" disabled={busy}>
      {busy ? "Saving…" : editing ? "Save changes" : "Add goal"}
    </button>
    {#if editing && onDismiss}
      <button type="button" class="goal-form__dismiss" onclick={onDismiss}>
        Cancel
      </button>
    {/if}
  </div>
</form>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .field span {
    font-weight: 600;
  }

  .pair {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .pair .field {
    flex: 1 1 12rem;
  }

  input[type="text"],
  select,
  textarea,
  input[type="date"] {
    padding: 0.5rem 0.7rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 8px;
    font: inherit;
    background: var(--card, #fff);
    color: var(--fg, #1c2430);
  }

  small {
    color: var(--muted, #5b6675);
  }

  .goal-form__error {
    color: #b3261e;
    margin: 0;
  }

  .actions {
    display: flex;
    gap: 0.6rem;
  }

  button {
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

  .goal-form__dismiss {
    background: transparent;
    color: var(--fg, #1c2430);
    border: 1px solid var(--line, #e3e7ec);
  }
</style>
