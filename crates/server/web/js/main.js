import { fetchSummary } from './api.js';
import { filterRows, RANGES, rangeStart, sum } from './aggregate.js';
import { h, mount } from './dom.js';
import { daysBetween, timeAgo } from './format.js';
import { agentLabel, logo } from './icons.js';
import { renderAgents } from './views/agents.js';
import { onResize } from './views/bars.js';
import { renderComposition } from './views/composition.js';
import { renderDays } from './views/days.js';
import { renderHeatmap } from './views/heatmap.js';
import { renderKpis } from './views/kpis.js';
import { renderModels } from './views/models.js';
import { renderSpending } from './views/spending.js';
import { renderTrend } from './views/trend.js';

const REFRESH_MS = 30_000;
const STORAGE_KEY = 'tokens.ui';

const ui = {
    range: 'all',
    agent: 'all',
    metric: 'tokens',
    models: { sort: { key: 'total_tokens', dir: -1 }, open: null },
    days: { daysShown: null },
    spending: { shown: null },
    ...restore(),
};

let data = null;

function restore() {
    try {
        const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}');
        const restored = {};
        if (RANGES.some((range) => range.key === saved.range)) restored.range = saved.range;
        if (typeof saved.agent === 'string') restored.agent = saved.agent;
        if (saved.metric === 'cost' || saved.metric === 'tokens') restored.metric = saved.metric;
        return restored;
    } catch {
        return {};
    }
}

function persist() {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify({ range: ui.range, agent: ui.agent, metric: ui.metric }));
    } catch {
        /* storage unavailable: preferences just don't stick */
    }
}

function segmented(selector, options, current, choose) {
    mount(selector, options.map((option) => h('button', {
        type: 'button',
        'aria-pressed': String(option.key === current),
        onclick: () => choose(option.key),
    }, option.icon ?? null, option.label)));
}

function renderFilters(agents) {
    segmented('#range-filter', RANGES, ui.range, (key) => update({ range: key }));
    segmented('#agent-filter', [
        { key: 'all', label: 'All agents' },
        ...agents.map((agent) => ({ key: agent, label: agentLabel(agent), icon: logo(agent, { small: true }) })),
    ], ui.agent, (key) => update({ agent: key }));
    segmented('#trend-metric', [{ key: 'tokens', label: 'Tokens' }, { key: 'cost', label: 'Cost' }], ui.metric, (key) => update({ metric: key }));
}

function update(changes) {
    Object.assign(ui, changes);
    if ('range' in changes || 'agent' in changes) {
        ui.days.daysShown = null;
        ui.spending.shown = null;
    }
    persist();
    render();
}

function render() {
    if (!data) return;

    const rows = data.daily;
    const agents = [...new Set(rows.map((row) => row.agent))].sort();
    if (ui.agent !== 'all' && !agents.includes(ui.agent)) ui.agent = 'all';

    const today = data.today;
    const firstDay = rows.length ? rows[0].day : today;
    const start = rangeStart(ui.range, today, firstDay);
    const range = RANGES.find((item) => item.key === ui.range);
    const rangeLabel = range.days ? `last ${range.days} days` : 'all time';

    const scoped = filterRows(rows, { start, end: today, agent: ui.agent });
    const agentRows = ui.agent === 'all' ? rows : rows.filter((row) => row.agent === ui.agent);
    const rangeTotals = sum(scoped);

    renderFilters(agents);
    renderKpis({
        range: rangeTotals,
        today: sum(agentRows.filter((row) => row.day === today)),
        days: daysBetween(start, today) + 1,
    });
    renderComposition(rangeTotals, rangeLabel);
    renderAgents(filterRows(rows, { start, end: today, agent: 'all' }));
    renderSpending({
        payments: data.payments ?? [],
        start: range.days ? start : null,
        end: today,
        agent: ui.agent,
        apiCost: rangeTotals.cost_usd,
        rangeLabel,
        ui: ui.spending,
        onChange: render,
    });
    renderTrend({ rows: scoped, start, end: today, metric: ui.metric });
    renderHeatmap(agentRows, today);
    renderModels({ rows: scoped, start, end: today, ui: ui.models, onChange: render });
    renderDays({ rows: scoped, ui: ui.days, onChange: render });

    mount('#footer-meta', [
        `${data.source_count} source${data.source_count === 1 ? '' : 's'}`,
        `last sync ${timeAgo(data.last_sync_at)}`,
        data.timezone,
    ].join(' · '));
}

async function refresh() {
    const status = document.querySelector('#status');
    try {
        data = await fetchSummary();
        render();
        status.classList.remove('is-error');
        status.textContent = `Updated ${new Date(data.generated_at).toLocaleTimeString()}`;
    } catch (error) {
        status.classList.add('is-error');
        status.textContent = data ? 'Update failed, showing last data' : 'Could not load usage, retrying';
        console.error(error);
    }
}

onResize(document.querySelector('#trend'), render);
refresh();
setInterval(() => {
    if (document.visibilityState === 'visible') refresh();
}, REFRESH_MS);
document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') refresh();
});
