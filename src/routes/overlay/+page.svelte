<script lang="ts">
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import IconBolt from '@tabler/icons-svelte-runes/icons/bolt';
  import IconCamera from '@tabler/icons-svelte-runes/icons/camera';
  import IconPlayerRecord from '@tabler/icons-svelte-runes/icons/player-record';
  import IconPlayerPlay from '@tabler/icons-svelte-runes/icons/player-play';
  import IconPlayerPause from '@tabler/icons-svelte-runes/icons/player-pause';
  import IconAlertTriangle from '@tabler/icons-svelte-runes/icons/alert-triangle';
  import { applyAccent } from '$lib/accents';
  import { translate, type TKey } from '$lib/i18n';
  import { duration } from '$lib/format';
  import type { Toast } from '$lib/types';

  let toast = $state<Toast | null>(null);
  let seq = $state(0);

  onMount(() => {
    document.documentElement.classList.add('overlay');
    const un = listen<Toast>('overlay://toast', (e) => {
      toast = e.payload;
      applyAccent(e.payload.accent);
      seq++;
    });
    return () => {
      un.then((f) => f());
    };
  });

  const icons: Record<string, typeof IconBolt> = {
    clip: IconBolt,
    recording: IconPlayerRecord,
    'recording-start': IconPlayerRecord,
    screenshot: IconCamera,
    'replay-on': IconPlayerPlay,
    'replay-off': IconPlayerPause,
    'replay-off-hint': IconPlayerPause,
    error: IconAlertTriangle,
  };

  let t = $derived((k: string) => translate(toast?.lang ?? 'en', k as TKey));
  let title = $derived(toast ? t(`ov.${toast.kind}`) : '');
  let sub = $derived.by(() => {
    if (!toast) return '';
    if (toast.kind === 'error') return toast.message;
    if (toast.kind === 'replay-off-hint') return t('ov.replay-off-hint.sub');
    const parts = [];
    if (toast.seconds > 0) parts.push(duration(toast.seconds));
    if (toast.game) parts.push(toast.game);
    return parts.join(' · ');
  });
  let Icon = $derived(toast ? (icons[toast.kind] ?? IconBolt) : IconBolt);
</script>

{#key seq}
  {#if toast}
    <div class="wrap {toast.corner}">
      <div class="toast" class:error={toast.kind === 'error'} class:rec={toast.kind === 'recording-start'}>
        <div class="icon"><Icon size={22} stroke={2.2} /></div>
        <div class="text">
          <div class="title">{title}</div>
          {#if sub}<div class="sub">{sub}</div>{/if}
        </div>
        <div class="bar"></div>
      </div>
    </div>
  {/if}
{/key}

<style>
  .wrap {
    position: fixed;
    inset: 0;
    display: flex;
    padding: 10px;
    align-items: flex-start;
    justify-content: flex-end;
  }
  .wrap.top-left,
  .wrap.bottom-left {
    justify-content: flex-start;
  }
  .wrap.bottom-left,
  .wrap.bottom-right {
    align-items: flex-end;
  }
  .toast {
    --dir: 1;
    position: relative;
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 250px;
    max-width: 100%;
    padding: 14px 20px 14px 14px;
    border-radius: 18px;
    background: rgba(18, 13, 32, 0.94);
    border: 1px solid rgba(196, 181, 253, 0.16);
    box-shadow:
      0 18px 40px -12px rgba(0, 0, 0, 0.7),
      0 0 0 1px rgba(0, 0, 0, 0.3);
    overflow: hidden;
    animation:
      enter 0.45s var(--ease) both,
      leave 0.35s ease-in 2.75s forwards;
  }
  .top-left .toast,
  .bottom-left .toast {
    --dir: -1;
  }
  .toast::before {
    content: '';
    position: absolute;
    inset: 0;
    background: radial-gradient(120% 140% at 0% 0%, color-mix(in srgb, var(--accent-a) 22%, transparent), transparent 60%);
    pointer-events: none;
  }
  .icon {
    position: relative;
    width: 44px;
    height: 44px;
    flex-shrink: 0;
    border-radius: 13px;
    display: grid;
    place-items: center;
    background: var(--accent-grad);
    color: var(--accent-ink);
    animation: pop 0.5s 0.15s var(--ease) both;
  }
  .error .icon {
    background: linear-gradient(135deg, #ff6b8b, #ff9f43);
  }
  .rec .icon {
    background: linear-gradient(135deg, #ff4d6d, #ff7aa2);
  }
  .text {
    position: relative;
    min-width: 0;
  }
  .title {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 15px;
    letter-spacing: -0.01em;
    white-space: nowrap;
  }
  .sub {
    color: var(--text-2);
    font-size: 13px;
    font-weight: 600;
    margin-top: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 3px;
    width: 100%;
    background: var(--accent-grad);
    transform-origin: left;
    animation: drain 2.9s linear forwards;
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateX(calc(var(--dir) * 40px)) scale(0.96);
    }
  }
  @keyframes leave {
    to {
      opacity: 0;
      transform: translateX(calc(var(--dir) * 24px));
    }
  }
  @keyframes pop {
    from {
      transform: scale(0.4) rotate(-20deg);
      opacity: 0;
    }
  }
  @keyframes drain {
    to {
      transform: scaleX(0);
    }
  }
</style>
