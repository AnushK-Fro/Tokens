import { CATEGORIES, categorySum } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, exact, percent, perMillion } from '../format.js';
import { mixBar } from './mix.js';

export function renderComposition(totals, rangeLabel) {
    const total = categorySum(totals);
    document.querySelector('#composition-sub').textContent = `What the tokens were, ${rangeLabel}`;

    if (total === 0) {
        mount('#composition', h('p', { class: 'empty', text: 'No usage in this range.' }));
        return;
    }

    const prompt = totals.input_tokens + totals.cache_read_tokens + totals.cache_creation_tokens;

    mount('#composition',
        mixBar(totals),
        h('ul', { class: 'mix-list' }, CATEGORIES.map((category) => h('li', { class: 'mix-item' },
            h('i', { class: 'swatch', style: { background: category.color } }),
            h('span', { class: 'mix-name' }, category.label, h('small', { text: category.hint })),
            h('span', { class: 'mix-value', title: exact(totals[category.key]), text: compact(totals[category.key]) }),
            h('span', { class: 'mix-share', text: percent(totals[category.key] / total) }),
        ))),
        h('div', { class: 'mix-notes' },
            note('Output / prompt', prompt ? `1 : ${Math.round(prompt / Math.max(totals.output_tokens, 1))}` : '—'),
            note('Reasoning', totals.reasoning_tokens ? compact(totals.reasoning_tokens) : '—'),
            note('Blended $ / 1M', perMillion(totals.cost_usd, totals.total_tokens)),
        ),
    );
}

function note(label, value) {
    return h('div', { class: 'mix-note' }, h('span', { class: 'label', text: label }), h('strong', { text: value }));
}
