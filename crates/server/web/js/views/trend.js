import { buckets, CATEGORIES } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, money, monthLabel, readableDay, shortDate, addDays } from '../format.js';
import { renderBars } from './bars.js';
import { categoryLines } from './mix.js';

const UNIT_LABEL = { day: 'Daily', week: 'Weekly', month: 'Monthly' };

function bucketTitle(unit, key, end) {
    if (unit === 'day') return readableDay(key);
    if (unit === 'month') return monthLabel(key);
    const last = addDays(key, 6) > end ? end : addDays(key, 6);
    return `${shortDate(key)} – ${shortDate(last)}`;
}

export function renderTrend({ rows, start, end, metric }) {
    const { unit, items } = buckets(rows, start, end);

    document.querySelector('#trend-sub').textContent = `${UNIT_LABEL[unit]} ${metric === 'cost' ? 'API-equivalent cost' : 'tokens by type'}`;
    mount('#trend-legend', metric === 'cost'
        ? h('span', null, h('i', { class: 'swatch', style: { background: 'var(--heat-3)' } }), 'Cost (USD)')
        : CATEGORIES.map((category) => h('span', null, h('i', { class: 'swatch', style: { background: category.color } }), category.label)));

    const container = document.querySelector('#trend');
    renderBars(container, {
        items: items.map(({ key, totals }) => ({
            key,
            segments: metric === 'cost'
                ? [{ value: totals.cost_usd, color: 'var(--heat-3)' }]
                : CATEGORIES.map((category) => ({ value: totals[category.key], color: category.color })),
            tip: () => ({
                title: bucketTitle(unit, key, end),
                lines: metric === 'cost'
                    ? [['Cost', money(totals.cost_usd)], ['Tokens', compact(totals.total_tokens)]]
                    : categoryLines(totals, [['Total', compact(totals.total_tokens)], ['Cost', money(totals.cost_usd)]]),
            }),
        })),
        format: metric === 'cost' ? (value) => (value >= 1000 ? `$${compact(value)}` : money(value).replace('.00', '')) : compact,
        tickLabel: (item) => (unit === 'month' ? monthLabel(item.key) : shortDate(item.key)),
    });
}
