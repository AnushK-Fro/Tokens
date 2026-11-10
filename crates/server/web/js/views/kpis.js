import { cacheHitRate } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, exact, money, percent } from '../format.js';

function tile(label, value, detail, title) {
    return h('article', { class: 'kpi', title },
        h('p', { class: 'label', text: label }),
        h('p', { class: 'kpi-value', text: value }),
        detail ? h('p', { class: 'kpi-detail', text: detail }) : null,
    );
}

/** Four headline numbers for the selected range; everything finer lives in the panels below. */
export function renderKpis({ range, today, days }) {
    mount('#kpis',
        tile('Tokens', compact(range.total_tokens), null, `${exact(range.total_tokens)} tokens`),
        tile('Cost', money(range.cost_usd), days > 1 ? `${money(range.cost_usd / days)} / day` : null),
        tile('Today', money(today.cost_usd), `${compact(today.total_tokens)} tokens`),
        tile('Cache hit', percent(cacheHitRate(range)), null, 'Share of prompt tokens read from cache'),
    );
}
