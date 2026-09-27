<script lang="ts" module>
  import { createContext } from 'svelte';

  /** Element ids of the row's label and hint, for aria-labelledby/-describedby. */
  export type RowIds = { label: string; hint: string | undefined };
  const [getRow, setRow, hasRow] = createContext<RowIds>();

  /** The settings row a control sits in (null outside one): the control
   *  names itself after the row's label. Call during component init. */
  export const rowIds = (): RowIds | null => (hasRow() ? getRow() : null);
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  let { label, hint = '', children }: { label: string; hint?: string; children: Snippet } = $props();

  const id = $props.id();
  setRow({
    label: `${id}-label`,
    get hint() {
      return hint ? `${id}-hint` : undefined;
    },
  });
</script>

<div class="row">
  <div class="text">
    <div class="label" id="{id}-label">{label}</div>
    {#if hint}<div class="hint" id="{id}-hint">{hint}</div>{/if}
  </div>
  <div class="ctl">{@render children()}</div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    min-height: 52px;
    padding: 10px 0;
    border-top: 1px solid var(--line);
  }
  .row:first-of-type {
    border-top: none;
  }
  .label {
    font-size: 13.5px;
    font-weight: 500;
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
    margin-top: 1px;
    max-width: 420px;
    overflow-wrap: anywhere;
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
