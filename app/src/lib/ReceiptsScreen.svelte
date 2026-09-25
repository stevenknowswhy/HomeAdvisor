<script lang="ts">
  import { onMount } from "svelte";
  import ReceiptRow from "./ReceiptRow.svelte";
  import {
    errorMessage,
    fetchEgressReceipts,
    RECEIPTS_PAGE_SIZE,
    type EgressReceiptsInput,
    type ReceiptView,
  } from "./ipc";
  import { describeLayer1, describeScan, scanSummaryText } from "./verdicts";

  // The family-facing transparency surface: the append-only egress log,
  // rendered read-only, one keyset page at a time. Receipts are facts about
  // the gate — what was gated, its hash, both verdicts, the decision — so
  // this screen only ever reads through the audited `egress_receipts`
  // command and renders; it has no edit, delete, or retry control, and
  // none may be added without breaking the read-only test. The load-more
  // button is a read control: it asks the core for the next page.
  let receipts = $state<ReceiptView[] | null>(null);
  let error = $state<string | null>(null);
  let loadingMore = $state(false);
  let loadError = $state<string | null>(null);

  /** Length of the most recent page. A full page means more rows may
   *  exist; a short one was the last. (An exactly-full log costs one
   *  extra empty fetch before the control hides — accepted for the sake
   *  of not counting the whole table on every read.) */
  let lastPageCount = $state(0);

  const hasMore = $derived(
    receipts !== null && lastPageCount === RECEIPTS_PAGE_SIZE,
  );

  onMount(async () => {
    try {
      const page = await fetchEgressReceipts({
        beforeCreatedAt: null,
        beforeId: null,
      });
      receipts = page;
      lastPageCount = page.length;
    } catch (e) {
      error = errorMessage(e);
    }
  });

  async function loadMore(): Promise<void> {
    if (receipts === null || loadingMore) return;
    const last = receipts[receipts.length - 1];
    if (last === undefined) return;
    loadingMore = true;
    loadError = null;
    try {
      const cursor: EgressReceiptsInput = {
        beforeCreatedAt: last.createdAt,
        beforeId: last.id,
      };
      const page = await fetchEgressReceipts(cursor);
      receipts = [...receipts, ...page];
      lastPageCount = page.length;
    } catch (e) {
      // A failed page keeps every receipt already loaded on screen.
      loadError = errorMessage(e);
    } finally {
      loadingMore = false;
    }
  }
</script>

<section aria-labelledby="receipts-heading">
  <h2 id="receipts-heading">Privacy receipts</h2>
  <p class="section-hint">
    Every attempt to move your family's data off this device — allowed or
    blocked — is recorded here. The log is append-only: receipts cannot be
    edited or deleted.
  </p>

  {#if error !== null}
    <p class="receipts-error" role="alert" data-testid="receipts-error">
      The receipt log could not be read: {error}
    </p>
  {:else if receipts === null}
    <p
      class="receipts-loading"
      data-testid="receipts-loading"
      aria-live="polite"
    >
      Reading your store…
    </p>
  {:else if receipts.length === 0}
    <div class="receipts-empty" data-testid="receipts-empty">
      <p class="empty-title">Nothing has ever tried to leave this device.</p>
      <p class="empty-body">
        This log fills in the moment any egress is attempted — allowed or
        blocked — with the payload's hash and what the privacy gate did.
      </p>
    </div>
  {:else}
    <table data-testid="receipts-table" id="receipts-table">
      <caption class="visually-hidden">
        Append-only egress log receipts
      </caption>
      <thead>
        <tr>
          <th scope="col">Purpose</th>
          <th scope="col">Decision</th>
          <th scope="col">Payload hash</th>
          <th scope="col">Layer 1</th>
          <th scope="col">Scan</th>
          <th scope="col">Reason</th>
        </tr>
      </thead>
      <tbody>
        {#each receipts as receipt (receipt.id)}
          <ReceiptRow
            purpose={receipt.purpose}
            decision={receipt.decision}
            payloadHash={receipt.payloadHash}
            createdAt={receipt.createdAt}
            layer1Verdict={describeLayer1(receipt.layer1Verdict)}
            scanSummary={scanSummaryText(describeScan(receipt.layaScanJson))}
            scanDetail={receipt.layaModelVersion}
            reason={receipt.reason ?? undefined}
          />
        {/each}
      </tbody>
    </table>
    {#if loadError !== null || hasMore}
      <div class="receipts-more">
        {#if loadError !== null}
          <p
            class="receipts-error"
            role="alert"
            data-testid="receipts-load-error"
          >
            The next page of receipts could not be read: {loadError}
          </p>
        {/if}
        {#if hasMore}
          <button
            type="button"
            data-testid="receipts-load-more"
            onclick={loadMore}
            disabled={loadingMore}
            aria-controls="receipts-table"
          >
            {loadingMore ? "Loading more receipts…" : "Show more receipts"}
          </button>
        {/if}
      </div>
    {/if}
  {/if}
</section>

<style>
  section {
    background: var(--card, #ffffff);
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 12px;
    padding: 1.25rem 1.5rem 1.5rem;
  }

  h2 {
    margin: 0 0 0.25rem;
    font-size: 1.15rem;
  }

  .section-hint {
    color: var(--muted, #5b6675);
    font-size: 0.9rem;
    margin: 0 0 1rem;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }

  th {
    text-align: left;
    padding: 0.4rem 0.75rem;
    color: var(--muted, #5b6675);
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

  .receipts-empty,
  .receipts-loading {
    border: 1px dashed var(--line, #e3e7ec);
    border-radius: 10px;
    padding: 1.25rem 1rem;
  }

  .empty-title {
    margin: 0 0 0.3rem;
    font-weight: 600;
  }

  .empty-body {
    margin: 0;
    color: var(--muted, #5b6675);
  }

  .receipts-error {
    margin: 0;
  }

  .receipts-more {
    margin-top: 0.75rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .receipts-more button {
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 8px;
    background: transparent;
    color: var(--muted, #5b6675);
    font: inherit;
    font-size: 0.85rem;
    padding: 0.4rem 1rem;
    cursor: pointer;
  }

  .receipts-more button:hover:not(:disabled) {
    background: var(--line, #e3e7ec);
  }

  .receipts-more button:disabled {
    cursor: default;
    opacity: 0.7;
  }
</style>
