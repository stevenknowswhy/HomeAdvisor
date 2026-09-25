<script lang="ts">
  import BandPicker from "./lib/BandPicker.svelte";
  import ReceiptRow from "./lib/ReceiptRow.svelte";

  // Demo data, shaped like the real thing: band options come from the seeded
  // v1 taxonomy (migration 002 in ha-store), region classes from the household
  // CHECK, and receipts from the egress_log columns. Nothing here calls the
  // backend — IPC reads land in a later slice.
  const childAgeOptions = [
    { value: "age_0_2", label: "0-2" },
    { value: "age_3_5", label: "3-5" },
    { value: "age_6_9", label: "6-9" },
    { value: "age_10_12", label: "10-12" },
    { value: "age_13_15", label: "13-15" },
    { value: "age_16_17", label: "16-17" },
  ];

  const adultAgeOptions = [
    { value: "age_25_34", label: "25-34" },
    { value: "age_35_44", label: "35-44" },
    { value: "age_45_54", label: "45-54" },
  ];

  const incomeOptions = [
    { value: "income_under_50k", label: "under-50k" },
    { value: "income_50k_75k", label: "50k-75k" },
    { value: "income_75k_100k", label: "75k-100k" },
    { value: "income_100k_150k", label: "100k-150k" },
    { value: "income_150k_200k", label: "150k-200k" },
    { value: "income_200k_plus", label: "200k+" },
  ];

  const regionOptions = [
    { value: "urban_metro", label: "Urban metro" },
    { value: "suburban", label: "Suburban" },
    { value: "rural", label: "Rural" },
    { value: "small_town", label: "Small town" },
  ];

  const receipts = [
    {
      purpose: "domain research — wealth",
      decision: "ALLOW",
      payloadHash:
        "9f2c7a1e5b8d4f3a6c0e2d4b6a8c0e2d4b6a8c0e2d4b6a8c0e2d4b6a8c0e2d4b",
      createdAt: "2026-09-24 09:14:02Z",
      reason: undefined,
    },
    {
      purpose: "domain research — education",
      decision: "QUARANTINE",
      payloadHash:
        "4c8b1d0f7a3e9c5b1f8d2a6e0c4b8f2d6a0c4b8f2d6a0e4c8b1d0f7a3e9c5b1f8d",
      createdAt: "2026-09-24 11:40:17Z",
      reason: "leak class above threshold: unique combination",
    },
    {
      purpose: "domain research — health",
      decision: "BLOCK",
      payloadHash:
        "2e6a0c4b8f2d6a0e4c8b1d0f7a3e9c5b1f8d2a6e0c4b8f2d6a0e4c8b1d0f7a3e",
      createdAt: "2026-09-24 18:22:55Z",
      reason: "sidecar unavailable — fail closed",
    },
  ];

  let childAgeBand = $state<string | null>(null);
  let adultAgeBand = $state<string | null>("age_35_44");
  let incomeBand = $state<string | null>(null);
  let regionClass = $state<string | null>("urban_metro");

  function bandLabel(
    options: { value: string; label: string }[],
    value: string | null,
  ): string {
    return options.find((option) => option.value === value)?.label ?? "—";
  }
</script>

<main>
  <header>
    <h1>Home Advisor</h1>
    <p>
      A few sharp actions a day for your family, computed on your device.
      Nothing leaves this machine without passing the privacy gate.
    </p>
    <p class="scaffold-note">
      Scaffold preview — static demo data; Tauri IPC reads arrive in the next
      slice.
    </p>
  </header>

  <section aria-labelledby="profile-heading">
    <h2 id="profile-heading">Household profile (demo)</h2>
    <p class="section-hint">
      Bands, not raw values: the store holds band foreign keys only, so exact
      ages and incomes are unrepresentable by design.
    </p>
    <div class="pickers">
      <BandPicker
        id="child-age-band"
        label="Child age band"
        options={childAgeOptions}
        value={childAgeBand}
        onSelect={(value) => (childAgeBand = value)}
      />
      <BandPicker
        id="adult-age-band"
        label="Adult age band"
        options={adultAgeOptions}
        value={adultAgeBand}
        onSelect={(value) => (adultAgeBand = value)}
      />
      <BandPicker
        id="income-band"
        label="Household income band"
        options={incomeOptions}
        value={incomeBand}
        onSelect={(value) => (incomeBand = value)}
      />
      <BandPicker
        id="region-class"
        label="Region class"
        options={regionOptions}
        value={regionClass}
        onSelect={(value) => (regionClass = value)}
      />
    </div>
    <p class="selected-summary">
      Selected — child: {bandLabel(childAgeOptions, childAgeBand)} · adult:
      {bandLabel(adultAgeOptions, adultAgeBand)} · income:
      {bandLabel(incomeOptions, incomeBand)} · region:
      {bandLabel(regionOptions, regionClass)}
    </p>
  </section>

  <section aria-labelledby="receipts-heading">
    <h2 id="receipts-heading">Privacy receipts (demo)</h2>
    <p class="section-hint">
      Every outbound attempt leaves a receipt in the append-only egress log —
      payload hash, scan verdict, and the gate's decision.
    </p>
    <table>
      <caption class="visually-hidden">Egress log receipts</caption>
      <thead>
        <tr>
          <th scope="col">Purpose</th>
          <th scope="col">Decision</th>
          <th scope="col">Payload hash</th>
          <th scope="col">Recorded</th>
          <th scope="col">Reason</th>
        </tr>
      </thead>
      <tbody>
        {#each receipts as receipt (receipt.payloadHash)}
          <ReceiptRow
            purpose={receipt.purpose}
            decision={receipt.decision}
            payloadHash={receipt.payloadHash}
            createdAt={receipt.createdAt}
            reason={receipt.reason}
          />
        {/each}
      </tbody>
    </table>
  </section>
</main>

<style>
  :global(:root) {
    --bg: #f6f7f9;
    --fg: #1c2430;
    --card: #ffffff;
    --line: #e3e7ec;
    --muted: #5b6675;
    color-scheme: light dark;
    font-family: system-ui, sans-serif;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --bg: #14181e;
      --fg: #e8ecf1;
      --card: #1c222a;
      --line: #2b333e;
      --muted: #9aa7b5;
    }
  }

  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--fg);
  }

  main {
    max-width: 44rem;
    margin: 0 auto;
    padding: 2rem 1.5rem 4rem;
    display: flex;
    flex-direction: column;
    gap: 2rem;
  }

  header p {
    color: var(--muted);
    margin: 0.25rem 0 0;
  }

  h1 {
    margin: 0;
    font-size: 1.6rem;
  }

  h2 {
    margin: 0 0 0.25rem;
    font-size: 1.15rem;
  }

  section {
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 1.25rem 1.5rem 1.5rem;
  }

  .section-hint {
    color: var(--muted);
    font-size: 0.9rem;
    margin: 0 0 1rem;
  }

  .scaffold-note {
    font-size: 0.85rem;
  }

  .pickers {
    display: grid;
    gap: 0.75rem;
  }

  .selected-summary {
    margin: 0.75rem 0 0;
    font-size: 0.9rem;
    color: var(--muted);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
  }

  th {
    text-align: left;
    padding: 0.4rem 0.75rem;
    color: var(--muted);
    font-weight: 600;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
