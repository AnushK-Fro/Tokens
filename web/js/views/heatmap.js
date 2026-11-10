import { groupBy, sum } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { addDays, daysBetween, exact, money, parseDay, readableDay } from '../format.js';
import { LAUNCHES } from '../launches.js';
import { bindTooltips } from '../tooltip.js';

const WEEKS = 53;
const LABEL_CHAR = 6.1; // 10px Geist Mono advance, for label collision checks
const LABEL_GAP = 10;

/**
 * Launch markers: a hairline in the gutter left of the launch week, a small label above the grid
 * per launch (same-day launches share the line and stack their labels; others take a new row only
 * when two would touch), and a ring on the launch day's cell.
 */
function launchMarkers(container, start, today) {
    const width = container.clientWidth || 680;
    const gap = parseFloat(getComputedStyle(container).columnGap) || 3;
    const column = (width - (WEEKS - 1) * gap) / WEEKS;
    const rowEnds = [];

    const markers = LAUNCHES.filter((launch) => launch.day >= start && launch.day <= today).map((launch) => {
        const col = Math.floor(daysBetween(start, launch.day) / 7);
        const x = col * (column + gap) - gap / 2;
        const textWidth = launch.label.length * LABEL_CHAR;
        const flip = x + 5 + textWidth > width;
        const from = flip ? x - 5 - textWidth : x + 5;
        let row = rowEnds.findIndex((end) => from - LABEL_GAP >= end);
        if (row < 0) row = rowEnds.length;
        rowEnds[row] = from + textWidth;
        return { day: launch.day, col, row, flip, label: launch.label, name: launch.name };
    });

    return { markers, rows: rowEnds.length };
}

export function renderHeatmap(rows, today) {
    const container = document.querySelector('#heatmap');
    const totals = new Map([...groupBy(rows, (row) => row.day)].map(([day, items]) => [day, sum(items)]));
    const max = Math.max(1, ...[...totals.values()].map((value) => value.total_tokens));

    // Columns are Sunday-first weeks ending with the one containing today.
    const weekday = parseDay(today).getUTCDay();
    const start = addDays(today, -weekday - (WEEKS - 1) * 7);
    const { markers, rows: labelRows } = launchMarkers(container, start, today);
    const launchOn = new Map([...groupBy(markers, (marker) => marker.day)].map(([day, items]) => [day, items.map((item) => item.name).join(', ')]));
    const cells = [];

    for (let index = 0; index < WEEKS * 7; index++) {
        const day = addDays(start, index);
        if (day > today) {
            cells.push(h('span', { class: 'future' }));
            continue;
        }
        const tokens = totals.get(day)?.total_tokens ?? 0;
        const level = tokens === 0 ? 0 : Math.max(1, Math.ceil((Math.log10(tokens + 1) / Math.log10(max + 1)) * 4));
        cells.push(h('span', { class: launchOn.has(day) ? `l${level} launch-day` : `l${level}`, tabindex: -1, dataset: { tip: day } }));
    }

    // One line per day: same-day launches stack their labels on a single hairline.
    const lines = [...groupBy(markers, (marker) => marker.day).values()].map((group) => {
        const top = Math.min(...group.map((marker) => marker.row));
        const labels = group.map((marker) => {
            const label = h('b', { class: marker.flip ? 'launch-label flip' : 'launch-label', text: marker.label, dataset: { tip: marker.day } });
            // CSSOM, not a style attribute: the CSP (style-src 'self') blocks inline style attributes.
            label.style.setProperty('--stack', marker.row - top);
            return label;
        });
        const node = h('div', { class: 'launch' }, ...labels);
        node.style.setProperty('--col', group[0].col);
        node.style.setProperty('--row', top);
        return node;
    });

    container.classList.toggle('has-launches', markers.length > 0);
    container.style.setProperty('--launch-rows', labelRows);
    mount(container, cells, lines);

    if (!container.dataset.bound) {
        container.dataset.bound = '1';
        bindTooltips(container, (cell) => {
            const day = cell.dataset.tip;
            const value = container.__totals.get(day);
            const launch = container.__launches.get(day);
            return {
                title: readableDay(day),
                lines: [
                    ...(value ? [['Tokens', exact(value.total_tokens)], ['Cost', money(value.cost_usd)]] : [['No usage', '']]),
                    ...(launch ? [['Launch', launch]] : []),
                ],
            };
        });
    }
    container.__totals = totals;
    container.__launches = launchOn;
}
