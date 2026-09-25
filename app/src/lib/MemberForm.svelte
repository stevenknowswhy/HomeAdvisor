<script lang="ts">
  /**
   * Add-member form. Sensitive attributes are band picks only: the age
   * never appears as a number, the school stage is a pick from the v1
   * stages, and the display name is the sole free-text field — it is
   * quarantined (`*_local`) in the store and never leaves the device.
   */
  import * as api from "./api";
  import { errorMessage } from "./api";
  import {
    ADULT_AGE_BANDS,
    CHILD_AGE_BANDS,
    SCHOOL_STAGES,
    bandLabel,
  } from "./bands";
  import BandPicker from "./BandPicker.svelte";

  interface Props {
    householdId: string;
    onAdded: (member: api.MemberView) => void;
  }

  let { householdId, onAdded }: Props = $props();

  let role = $state<"adult" | "child">("adult");
  let displayName = $state("");
  let ageBand = $state<string | null>(null);
  let schoolStage = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  let ageOptions = $derived(
    role === "child" ? CHILD_AGE_BANDS : ADULT_AGE_BANDS,
  );

  function chooseRole(next: "adult" | "child") {
    role = next;
    // Bands belong to a role; a stale pick from the other list must not
    // survive the switch.
    ageBand = null;
    schoolStage = null;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!ageBand) {
      error = "Pick an age band for this member.";
      return;
    }
    busy = true;
    error = null;
    try {
      const member = await api.addMember({
        household_id: householdId,
        role,
        display_name: displayName.trim() || null,
        age_band_id: ageBand,
        school_stage: role === "child" ? schoolStage : null,
      });
      displayName = "";
      ageBand = null;
      schoolStage = null;
      onAdded(member);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<form class="member-form" onsubmit={submit}>
  <fieldset>
    <legend>Role</legend>
    <label>
      <input
        type="radio"
        name="member-role"
        value="adult"
        checked={role === "adult"}
        onchange={() => chooseRole("adult")}
      />
      Adult
    </label>
    <label>
      <input
        type="radio"
        name="member-role"
        value="child"
        checked={role === "child"}
        onchange={() => chooseRole("child")}
      />
      Child
    </label>
  </fieldset>

  <label class="field">
    <span>Name or nickname</span>
    <input
      type="text"
      bind:value={displayName}
      placeholder="What your family calls them"
      aria-label="Member display name"
    />
    <small>Stored only on this device — never included in anything the app sends out.</small>
  </label>

  <BandPicker
    id="member-age-band"
    label="Age band"
    options={ageOptions}
    value={ageBand}
    onSelect={(value) => (ageBand = value)}
  />

  {#if role === "child"}
    <BandPicker
      id="member-school-stage"
      label="School stage"
      options={SCHOOL_STAGES}
      value={schoolStage}
      onSelect={(value) => (schoolStage = value)}
    />
  {/if}

  {#if error}
    <p class="member-form__error" role="alert">{error}</p>
  {/if}

  <button type="submit" disabled={busy}>
    {busy ? "Adding…" : `Add ${role === "child" ? "child" : "adult"}`}
  </button>
  <p class="member-form__summary">
    Adding {role === "child" ? "a child" : "an adult"} — age band{" "}
    {bandLabel(ageOptions, ageBand)}
    {#if role === "child" && schoolStage}
      · {bandLabel(SCHOOL_STAGES, schoolStage)}
    {/if}
  </p>
</form>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  fieldset {
    display: flex;
    gap: 1rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 10px;
    padding: 0.5rem 1rem;
    margin: 0;
  }

  legend {
    font-weight: 600;
    padding: 0 0.4rem;
  }

  fieldset label {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
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

  small {
    color: var(--muted, #5b6675);
  }

  .member-form__error {
    color: #b3261e;
    margin: 0;
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

  .member-form__summary {
    color: var(--muted, #5b6675);
    margin: 0;
  }
</style>
