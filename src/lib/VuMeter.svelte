<script lang="ts">
    let { peak = 0 } = $props();

    // Visual: 16 segments, light up proportionally to peak.
    // Smooth decay so the bar doesn't flicker.
    const SEGMENTS = 16;
    let displayed = $state(0);
    let raf: number | null = null;

    $effect(() => {
        const target = Math.min(1, Math.max(0, peak));
        if (raf) cancelAnimationFrame(raf);
        const animate = () => {
            const diff = target - displayed;
            // Rise quickly, decay slowly.
            const step = diff > 0 ? diff * 0.6 : diff * 0.15;
            displayed = Math.max(0, displayed + step);
            if (Math.abs(target - displayed) > 0.005) {
                raf = requestAnimationFrame(animate);
            }
        };
        animate();
    });

    let activeCount = $derived(Math.round(displayed * SEGMENTS));
</script>

<div class="vu" aria-label="Audio input level">
    {#each Array(SEGMENTS) as _, i}
        <span
            class="seg"
            class:on={i < activeCount}
            class:hot={i >= SEGMENTS * 0.75}
        ></span>
    {/each}
</div>

<style>
    .vu {
        display: flex;
        gap: 2px;
        align-items: stretch;
        height: 14px;
        width: 100%;
        max-width: 240px;
    }
    .seg {
        flex: 1;
        background: var(--border);
        border-radius: 2px;
        opacity: 0.5;
        transition: background 80ms linear, opacity 80ms linear;
    }
    .seg.on {
        opacity: 1;
        background: var(--info);
    }
    .seg.on.hot {
        background: var(--accent);
    }
</style>
