import { groupBy, sum } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, exact, money, readableDay } from '../format.js';
import { agentLabel, modelLogo } from '../icons.js';
import { bindTooltips } from '../tooltip.js';
import { categoryLines, mixBar } from './mix.js';

const PAGE = 7;

/** Tooltip content for the bars currently on screen, indexed by `data-tip`. */
let tips = [];

/** `ui.daysShown` persists the "show more" depth across refreshes. */
export function renderDays({ rows, ui, onChange }) {
    const container = document.querySelector('#days');
    const days = [...groupBy(rows, (row) => row.day)].sort(([a], [b]) => (a < b ? 1 : -1));
    const shown = days.slice(0, ui.daysShown ?? PAGE);

    if (!container.dataset.bound) {
        container.dataset.bound = '1';
        bindTooltips(container, (node) => tips[Number(node.dataset.tip)]);
    }

    if (!days.length) {
        mount(container, h('p', { class: 'empty', text: 'No usage in this range.' }));
        return;
    }

    tips = [];

    mount(container,
        shown.map(([day, items]) => {
            const totals = sum(items);
            items.sort((a, b) => b.total_tokens - a.total_tokens);
            return h('article', { class: 'day' },
                h('header', { class: 'day-head' },
                    h('h3', { text: readableDay(day) }),
                    h('p', null, h('span', { text: `${compact(totals.total_tokens)} tokens` }), h('span', { class: 'cost', text: money(totals.cost_usd) })),
                ),
                items.map((row) => {
                    tips.push({
                        title: `${row.model} · ${readableDay(day)}`,
                        lines: categoryLines(row, [['Total', exact(row.total_tokens)], ['Cost', money(row.cost_usd)]]),
                    });
                    return h('div', { class: 'day-row' },
                        h('div', { class: 'day-model', title: agentLabel(row.agent) }, modelLogo(row.agent, row.model), h('span', { text: row.model })),
                        h('div', { class: 'day-mix', tabindex: 0, dataset: { tip: String(tips.length - 1) } }, mixBar(row)),
                        h('p', { class: 'day-tokens', title: exact(row.total_tokens), text: compact(row.total_tokens) }),
                        h('p', { class: 'day-cost', text: money(row.cost_usd) }),
                    );
                }),
            );
        }),
        shown.length < days.length
            ? h('button', {
                class: 'more',
                type: 'button',
                text: `Show ${Math.min(PAGE, days.length - shown.length)} more days`,
                onclick: () => {
                    ui.daysShown = shown.length + PAGE;
                    onChange();
                },
            })
            : null,
    );
}
