import { CATEGORIES, categorySum } from '../aggregate.js';
import { h } from '../dom.js';
import { exact, percent } from '../format.js';

/** Flat proportional bar of the four token categories. */
export function mixBar(totals) {
    const total = categorySum(totals);
    return h('div', { class: 'mix-bar', role: 'img', 'aria-label': describe(totals) },
        total > 0
            ? CATEGORIES.filter((category) => totals[category.key] > 0).map((category) => h('span', {
                style: { flexGrow: String(totals[category.key]), background: category.color },
            }))
            : null,
    );
}

export function describe(totals) {
    const total = categorySum(totals);
    return CATEGORIES.map((category) => `${category.label} ${percent(total ? totals[category.key] / total : 0)}`).join(', ');
}

/** Tooltip lines for a totals object. */
export function categoryLines(totals, extra = []) {
    return [
        ...CATEGORIES.map((category) => [category.label, exact(totals[category.key]), category.color]),
        ...(totals.reasoning_tokens ? [['of which reasoning', exact(totals.reasoning_tokens)]] : []),
        ...extra,
    ];
}
