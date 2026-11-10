import { byModel, buckets, cacheHitRate, CATEGORIES } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, exact, money, percent, perMillion, readableDay, shortDate } from '../format.js';
import { modelLogo, agentLabel } from '../icons.js';
import { renderBars } from './bars.js';
import { categoryLines, mixBar } from './mix.js';

const COLUMNS = [
    { key: 'model', label: 'Model', value: (m) => m.model.toLowerCase(), align: 'left' },
    { key: 'input_tokens', label: 'Input', value: (m) => m.totals.input_tokens },
    { key: 'output_tokens', label: 'Output', value: (m) => m.totals.output_tokens },
    { key: 'cache_read_tokens', label: 'Cache read', value: (m) => m.totals.cache_read_tokens },
    { key: 'cache_creation_tokens', label: 'Cache write', value: (m) => m.totals.cache_creation_tokens },
    { key: 'total_tokens', label: 'Total', value: (m) => m.totals.total_tokens },
    { key: 'share', label: 'Share', value: (m) => m.totals.total_tokens },
    { key: 'cache_hit', label: 'Cache hit', value: (m) => cacheHitRate(m.totals) || 0 },
    { key: 'per_million', label: '$ / 1M', value: (m) => (m.totals.total_tokens ? m.totals.cost_usd / m.totals.total_tokens : 0) },
    { key: 'cost_usd', label: 'Cost', value: (m) => m.totals.cost_usd },
];

/**
 * `ui` holds sort and the open row across refreshes: `{ sort: {key, dir}, open: key|null }`.
 * `onChange` re-renders after the user sorts or opens a row.
 */
export function renderModels({ rows, start, end, ui, onChange }) {
    const table = document.querySelector('#models');
    const models = byModel(rows);
    const total = models.reduce((sum, model) => sum + model.totals.total_tokens, 0);
    const column = COLUMNS.find((item) => item.key === ui.sort.key) ?? COLUMNS[5];
    models.sort((a, b) => {
        const left = column.value(a);
        const right = column.value(b);
        return (left < right ? -1 : left > right ? 1 : 0) * ui.sort.dir;
    });

    const head = h('thead', null, h('tr', null, COLUMNS.map((item) => h('th', {
        scope: 'col',
        'aria-sort': item.key === ui.sort.key ? (ui.sort.dir < 0 ? 'descending' : 'ascending') : null,
    }, h('button', {
        type: 'button',
        text: item.label,
        onclick: () => {
            ui.sort = { key: item.key, dir: item.key === ui.sort.key ? -ui.sort.dir : (item.key === 'model' ? 1 : -1) };
            onChange();
        },
    })))));

    const body = h('tbody');

    if (!models.length) {
        body.append(h('tr', null, h('td', { colspan: COLUMNS.length, class: 'empty', text: 'No usage in this range.' })));
    }

    for (const model of models) {
        const open = ui.open === model.key;
        const toggle = () => {
            ui.open = open ? null : model.key;
            onChange();
        };
        const share = total ? model.totals.total_tokens / total : 0;

        body.append(h('tr', {
            class: `model-row${open ? ' is-open' : ''}`,
            tabindex: 0,
            'aria-expanded': String(open),
            onclick: toggle,
            onkeydown: (event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    toggle();
                }
            },
        },
            h('td', null, h('span', { class: 'model-name', title: `${model.model} · ${agentLabel(model.agent)}` },
                h('span', { class: 'chev', text: open ? '−' : '+' }),
                modelLogo(model.agent, model.model),
                h('span', { class: 'model-label', text: model.model }),
            )),
            CATEGORIES.map((category) => h('td', { title: exact(model.totals[category.key]), text: compact(model.totals[category.key]) })),
            h('td', { class: 'total', title: exact(model.totals.total_tokens), text: compact(model.totals.total_tokens) }),
            h('td', null, h('span', { class: 'share-cell' },
                h('span', { class: 'bar' }, h('i', { style: { width: `${share * 100}%` } })),
                percent(share),
            )),
            h('td', { text: percent(cacheHitRate(model.totals)) }),
            h('td', { text: perMillion(model.totals.cost_usd, model.totals.total_tokens) }),
            h('td', { class: 'cost', text: money(model.totals.cost_usd) }),
        ));

        if (open) {
            const cell = h('td', { class: 'detail-cell', colspan: COLUMNS.length });
            body.append(h('tr', { class: 'detail-row' }, cell));
            requestAnimationFrame(() => renderDetail(cell, model, start, end));
        }
    }

    mount(table, head, body);
}

function renderDetail(cell, model, start, end) {
    const chart = h('div', { class: 'chart', style: { minHeight: '160px' } });
    const totals = model.totals;
    const avgPerDay = model.activeDays ? totals.total_tokens / model.activeDays : 0;

    mount(cell, h('div', { class: 'model-detail' },
        h('div', { class: 'detail-chart' },
            h('h3', { text: `${model.model} · tokens per period` }),
            chart,
            mixBar(totals),
        ),
        h('div', { class: 'detail-stats' },
            stat('Active days', String(model.activeDays), `${shortDate(model.firstDay)} → ${shortDate(model.lastDay)}`),
            stat('Avg / active day', compact(avgPerDay), money(model.activeDays ? totals.cost_usd / model.activeDays : 0)),
            stat('Peak day', compact(model.peak.total_tokens), readableDay(model.peak.day)),
            stat('Reasoning', totals.reasoning_tokens ? compact(totals.reasoning_tokens) : '—', totals.output_tokens ? `${percent(totals.reasoning_tokens / totals.output_tokens)} of output` : ''),
            stat('Cache hit', percent(cacheHitRate(totals)), `${compact(totals.cache_read_tokens)} read`),
            stat('Output share', percent(totals.total_tokens ? totals.output_tokens / totals.total_tokens : NaN), `${exact(totals.output_tokens)} tokens`),
        ),
    ));

    const { items, unit } = buckets(model.rows, start, end);
    renderBars(chart, {
        height: 160,
        items: items.map(({ key, totals: bucket }) => ({
            key,
            segments: CATEGORIES.map((category) => ({ value: bucket[category.key], color: category.color })),
            tip: () => ({ title: unit === 'day' ? readableDay(key) : `${unit} of ${shortDate(key)}`, lines: categoryLines(bucket, [['Cost', money(bucket.cost_usd)]]) }),
        })),
        format: compact,
        tickLabel: (item) => shortDate(item.key),
    });
}

function stat(label, value, detail) {
    return h('div', { class: 'detail-stat' }, h('span', { class: 'label', text: label }), h('strong', { text: value }), detail ? h('small', { text: detail }) : null);
}
