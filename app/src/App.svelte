<script lang="ts">
  import DailyView from "./lib/DailyView.svelte";
  import OnboardingView from "./lib/OnboardingView.svelte";
  import ReceiptsScreen from "./lib/ReceiptsScreen.svelte";
</script>

<main>
  <header>
    <h1>Home Advisor</h1>
    <p>
      A few sharp actions a day for your family, computed on your device.
      Nothing leaves this machine without passing the privacy gate.
    </p>
    <p class="scaffold-note">
      The Today view and the privacy receipts below read your encrypted store
      directly, and onboarding writes through the same audited surface.
      Nothing in this window leaves your machine.
    </p>
  </header>

  <DailyView />

  <OnboardingView />

  <ReceiptsScreen />
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

  .scaffold-note {
    font-size: 0.85rem;
  }
</style>
