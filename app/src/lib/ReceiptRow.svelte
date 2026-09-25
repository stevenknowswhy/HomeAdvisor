<script lang="ts">
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    purpose: string;
    decision: string;
    payloadHash: string;
    createdAt: string;
    /** Layer 1's verdict as display text (parsed by `verdicts.ts`). */
    layer1Verdict?: string;
    /** The semantic scan's verdict as display text (parsed by `verdicts.ts`). */
    scanSummary?: string;
    /** Which checker model produced the scan, when one ran. */
    scanDetail?: string;
    reason?: string;
  }

  let {
    purpose,
    decision,
    payloadHash,
    createdAt,
    layer1Verdict = "—",
    scanSummary = "—",
    scanDetail,
    reason,
  }: Props = $props();

  // Human-sized hash with the full value on hover — the receipts screen is for
  // the family, not for byte-for-byte audits (the log itself keeps the hash).
  const shortHash = $derived(
    payloadHash.length > 12 ? payloadHash.slice(0, 12) + "…" : payloadHash,
  );
</script>

<tr data-testid="receipt-row">
  <!-- Row header: the purpose identifies the row, so screen readers
       announce it while traversing the row's other cells. -->
  <th scope="row" class="receipt-row__purpose">
    {purpose}
    <div class="receipt-row__time">{createdAt}</div>
  </th>
  <td><StatusBadge status={decision} /></td>
  <td class="receipt-row__hash">
    <code title={payloadHash}>{shortHash}</code>
  </td>
  <td class="receipt-row__verdict">{layer1Verdict}</td>
  <td class="receipt-row__verdict">
    <span title={scanDetail ?? undefined}>{scanSummary}</span>
    {#if scanDetail}
      <div class="receipt-row__model">{scanDetail}</div>
    {/if}
  </td>
  <td class="receipt-row__reason">{reason ?? "—"}</td>
</tr>

<style>
  td,
  th.receipt-row__purpose {
    padding: 0.5rem 0.75rem;
    border-top: 1px solid var(--line, #e3e7ec);
    text-align: left;
    vertical-align: top;
  }

  th.receipt-row__purpose {
    /* The purpose reads as a cell, not a heading — the styling stays. */
    font-weight: inherit;
  }

  .receipt-row__hash code {
    font-size: 0.8rem;
  }

  .receipt-row__verdict {
    color: var(--muted, #5b6675);
    font-size: 0.8rem;
    max-width: 11rem;
  }

  .receipt-row__model {
    color: var(--muted, #5b6675);
    font-size: 0.72rem;
  }

  .receipt-row__time {
    color: var(--muted, #5b6675);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    font-size: 0.78rem;
    margin-top: 0.15rem;
  }

  .receipt-row__reason {
    color: var(--muted, #5b6675);
  }
</style>
