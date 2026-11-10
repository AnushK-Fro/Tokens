import { mount, s, h } from '../dom.js';
import { bindTooltips } from '../tooltip.js';

const AXIS_WIDTH = 48;
const AXIS_HEIGHT = 22;

function niceMax(value) {
    if (value <= 0) return 1;
    const magnitude = 10 ** Math.floor(Math.log10(value));
    const step = [1, 2, 2.5, 5, 10].find((candidate) => candidate * magnitude >= value);
    return step * magnitude;
}

/**
 * Stacked vertical bars. Each item: `{ label, segments: [{ value, color }], tip: () => ({title, lines}) }`.
 * Square ends and 2px surface gaps between bars and between segments, to keep it flat.
 */
export function renderBars(container, { items, height = 220, format, tickLabel }) {
    if (!items.length || items.every((item) => item.segments.every((segment) => !segment.value))) {
        mount(container, h('p', { class: 'empty', text: 'No usage in this range.' }));
        return;
    }

    const width = Math.max(300, container.clientWidth || 600);
    const plotWidth = width - AXIS_WIDTH;
    const plotHeight = height - AXIS_HEIGHT;
    const max = niceMax(Math.max(...items.map((item) => item.segments.reduce((total, segment) => total + segment.value, 0))));
    const band = plotWidth / items.length;
    const gap = band >= 6 ? 2 : 0;
    const barWidth = Math.max(1, band - gap);
    const y = (value) => plotHeight - (value / max) * plotHeight;

    const grid = [0, 0.25, 0.5, 0.75, 1].map((fraction) => {
        const value = max * fraction;
        return [
            s('line', { class: fraction === 0 ? 'base-line' : 'grid-line', x1: AXIS_WIDTH, x2: width, y1: y(value), y2: y(value) }),
            s('text', { class: 'axis-label', x: AXIS_WIDTH - 8, y: y(value) + 3, 'text-anchor': 'end', text: format(value) }),
        ];
    });

    const labelEvery = Math.max(1, Math.ceil(items.length / Math.floor(plotWidth / 72)));
    const bars = items.map((item, index) => {
        const x = AXIS_WIDTH + index * band + gap / 2;
        let top = plotHeight;
        const rects = [];

        for (const segment of item.segments) {
            if (!segment.value) continue;
            const segmentHeight = (segment.value / max) * plotHeight;
            const drawn = rects.length && segmentHeight > 3 ? segmentHeight - 2 : segmentHeight;
            top -= segmentHeight;
            rects.push(s('rect', { x, y: top, width: barWidth, height: Math.max(drawn, 0.5), fill: segment.color }));
        }

        const label = index % labelEvery === 0
            ? s('text', { class: 'axis-label', x: x + barWidth / 2, y: height - 6, 'text-anchor': 'middle', text: tickLabel(item) })
            : null;

        return [
            rects,
            label,
            s('rect', { class: 'hit', x: AXIS_WIDTH + index * band, y: 0, width: band, height: plotHeight, tabindex: 0, dataset: { tip: String(index) } }),
        ];
    });

    const svg = s('svg', { viewBox: `0 0 ${width} ${height}`, width, height, role: 'img', 'aria-label': 'Stacked bar chart' }, grid, bars);
    mount(container, svg);
    bindTooltips(svg, (node) => items[Number(node.dataset.tip)].tip());
}

/** Re-renders `render` when the container's width changes. */
export function onResize(container, render) {
    let lastWidth = container.clientWidth;
    let frame = 0;
    new ResizeObserver(() => {
        if (container.clientWidth === lastWidth) return;
        lastWidth = container.clientWidth;
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(render);
    }).observe(container);
}
