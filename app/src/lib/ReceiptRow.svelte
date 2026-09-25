<script lang="ts">
  import StatusBadge from "./StatusBadge.svelte";

  interface Props {
    purpose: string;
    decision: string;
    payloadHash: string;
    createdAt: string;
    reason?: string;
  }

  let { purpose, decision, payloadHash, createdAt, reason }: Props = $props();

  // Human-sized hash with the full value on hover — the receipts screen is for
  // the family, not for byte-for-byte audits (the log itself keeps the hash).
  const shortHash = $derived(
    payloadHash.length > 12 ? payloadHash.slice(0, 12) + "…" : payloadHash,
  );
</script>

<tr data-testid="receipt-row">
  <td class="receipt-row__purpose">{purpose}</td>
  <td><StatusBadge status={decision} /></td>
  <td class="receipt-row__hash"><code title={payloadHash}>{shortHash}</code></td>
  <td class="receipt-row__time">{createdAt}</td>
  <td class="receipt-row__reason">{reason ?? "—"}</td>
</tr>

<style>
  td {
    padding: 0.5rem 0.75rem;
    border-top: 1px solid var(--line, #e3e7ec);
    text-align: left;
    vertical-align: top;
  }

  .receipt-row__hash code {
    font-size: 0.8rem;
  }

  .receipt-row__time {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .receipt-row__reason {
    color: var(--muted, #5b6675);
  }
</style>
