<script lang="ts">
  let { status }: { status: string } = $props();

  // Variants mirror the decision strings the gate writes to egress_log.decision
  // (ha-privacy: ALLOW | QUARANTINE | BLOCK). Anything else renders neutral.
  const variants: Record<string, string> = {
    ALLOW: "status-badge--allow",
    QUARANTINE: "status-badge--warn",
    BLOCK: "status-badge--block",
  };

  const variant = $derived(variants[status] ?? "status-badge--neutral");
</script>

<span class="status-badge {variant}" data-status={status}>{status}</span>

<style>
  .status-badge {
    display: inline-flex;
    align-items: center;
    padding: 0.1rem 0.6rem;
    border-radius: 999px;
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    border: 1px solid transparent;
  }

  .status-badge--allow {
    color: #1a7f37;
    background: #e6f4ea;
    border-color: #b7dfc4;
  }

  .status-badge--block {
    color: #b3261e;
    background: #fdecea;
    border-color: #f5c6c2;
  }

  .status-badge--warn {
    color: #8a5b00;
    background: #fdf3e0;
    border-color: #f0dcb0;
  }

  .status-badge--neutral {
    color: #475467;
    background: #f2f4f7;
    border-color: #e0e3e8;
  }
</style>
