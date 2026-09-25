<script lang="ts">
  import { onMount } from "svelte";
  import {
    errorMessage,
    fetchDailyRecommendations,
    type RecommendationView,
  } from "./ipc";

  // The daily view reads the store through the audited read command and
  // renders what it holds — nothing more. `null` means the read is still
  // in flight; the empty array is the designed empty state (the
  // intelligence layer that serves recommendations is a later slice).
  let recommendations = $state<RecommendationView[] | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      recommendations = await fetchDailyRecommendations();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  const typeLabels: Record<string, string> = {
    advice: "advice",
    task: "task",
    automated_action: "automated action",
    watch: "watch",
    do_nothing: "no action",
  };
</script>

<section aria-labelledby="daily-heading">
  <h2 id="daily-heading">Today</h2>
  <p class="section-hint">
    A few sharp actions, each with the evidence behind it — at most three a
    day.
  </p>

  {#if error !== null}
    <p class="daily-error" role="alert" data-testid="daily-error">
      The daily view could not be read: {error}
    </p>
    <button class="retry" onclick={load}>Try again</button>
  {:else if loading || recommendations === null}
    <p class="daily-loading" data-testid="daily-loading" aria-live="polite">
      Reading your store…
    </p>
  {:else if recommendations.length === 0}
    <div class="daily-empty" data-testid="daily-empty">
      <p class="empty-title">Nothing needs your attention today.</p>
      <p class="empty-body">
        When your advisor finds a sharp, evidence-backed action for your
        family, it appears here — at most three a day, so the important
        things stay important.
      </p>
    </div>
  {:else}
    <ul class="daily-list" data-testid="daily-list">
      {#each recommendations as recommendation (recommendation.id)}
        <li>
          <article
            class="recommendation"
            data-testid="recommendation-card"
            aria-label={recommendation.title}
          >
            <header>
              <span class="chip">{recommendation.category}</span>
              <span class="chip">
                {typeLabels[recommendation.recommendationType] ??
                  recommendation.recommendationType}
              </span>
            </header>
            <h3>{recommendation.title}</h3>
            <p class="explanation">{recommendation.explanation}</p>
            {#if recommendation.whyMe || recommendation.whyNow}
              <dl class="why">
                {#if recommendation.whyMe}
                  <div>
                    <dt>Why this</dt>
                    <dd>{recommendation.whyMe}</dd>
                  </div>
                {/if}
                {#if recommendation.whyNow}
                  <div>
                    <dt>Why now</dt>
                    <dd>{recommendation.whyNow}</dd>
                  </div>
                {/if}
              </dl>
            {/if}
            <p class="meta">
              {#if recommendation.effortEstimate}
                <span>Effort: {recommendation.effortEstimate}</span>
              {/if}
              {#if recommendation.expectedBenefit}
                <span>Benefit: {recommendation.expectedBenefit}</span>
              {/if}
              {#if recommendation.confidence !== null}
                <span>
                  Confidence: {Math.round(recommendation.confidence * 100)}%
                </span>
              {/if}
            </p>
            {#if recommendation.evidence.length > 0}
              <div class="evidence">
                <h4>Evidence</h4>
                <ul data-testid="evidence-list">
                  {#each recommendation.evidence as item (item.id)}
                    <li>
                      <span class="chip">{item.sourceType}</span>
                      {#if item.sourceUrl}
                        <a
                          href={item.sourceUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          data-testid="evidence-link"
                        >
                          {item.sourceTitle ?? item.sourceUrl}</a
                        >
                      {:else}
                        <span>{item.sourceTitle ?? item.sourceType}</span>
                      {/if}
                      {#if item.publicationDate}
                        <span class="evidence-date">
                          · {item.publicationDate}
                        </span>
                      {/if}
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}
          </article>
        </li>
      {/each}
    </ul>
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

  header {
    display: flex;
    gap: 0.4rem;
    margin-bottom: 0.4rem;
  }

  .chip {
    display: inline-block;
    padding: 0.05rem 0.55rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--muted, #5b6675);
    background: var(--bg, #f6f7f9);
    border: 1px solid var(--line, #e3e7ec);
  }

  h3 {
    margin: 0 0 0.3rem;
    font-size: 1.05rem;
  }

  .explanation {
    margin: 0 0 0.6rem;
  }

  .why {
    display: grid;
    gap: 0.3rem;
    margin: 0 0 0.6rem;
  }

  .why dt {
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted, #5b6675);
  }

  .why dd {
    margin: 0;
    font-size: 0.92rem;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 1rem;
    margin: 0 0 0.6rem;
    font-size: 0.82rem;
    color: var(--muted, #5b6675);
  }

  .evidence h4 {
    margin: 0 0 0.25rem;
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted, #5b6675);
  }

  .evidence ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }

  .evidence li {
    font-size: 0.85rem;
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
  }

  .evidence a {
    color: inherit;
  }

  .evidence-date {
    color: var(--muted, #5b6675);
    font-size: 0.8rem;
  }

  .daily-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1rem;
  }

  .recommendation {
    border: 1px solid var(--line, #e3e7ec);
    border-radius: 10px;
    padding: 0.85rem 1rem;
  }

  .daily-empty,
  .daily-loading {
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

  .daily-error {
    margin: 0 0 0.5rem;
  }

  .retry {
    font: inherit;
    padding: 0.35rem 0.9rem;
    border-radius: 8px;
    border: 1px solid var(--line, #e3e7ec);
    background: var(--card, #ffffff);
    color: inherit;
    cursor: pointer;
  }
</style>
