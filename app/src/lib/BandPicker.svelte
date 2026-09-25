<script lang="ts">
  interface BandOption {
    value: string;
    label: string;
  }

  interface Props {
    /** Stable id — also the radio group name. */
    id: string;
    label: string;
    options: BandOption[];
    /** Currently selected band value; null = nothing picked yet. */
    value: string | null;
    onSelect: (value: string) => void;
  }

  let { id, label, options, value, onSelect }: Props = $props();
</script>

<fieldset class="band-picker">
  <legend>{label}</legend>
  <div class="band-picker__options">
    {#each options as option (option.value)}
      <label
        class="band-picker__option"
        class:band-picker__option--selected={value === option.value}
      >
        <input
          type="radio"
          name={id}
          value={option.value}
          checked={value === option.value}
          onchange={() => onSelect(option.value)}
        />
        <span>{option.label}</span>
      </label>
    {/each}
  </div>
</fieldset>

<style>
  .band-picker {
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 10px;
    padding: 0.75rem 1rem 1rem;
    margin: 0;
  }

  legend {
    font-weight: 600;
    padding: 0 0.4rem;
  }

  .band-picker__options {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .band-picker__option {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 999px;
    cursor: pointer;
  }

  .band-picker__option--selected {
    border-color: #2f7d6d;
    background: rgba(47, 125, 109, 0.12);
  }

  input {
    margin: 0;
    accent-color: #2f7d6d;
  }
</style>
