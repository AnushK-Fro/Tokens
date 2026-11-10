import { addDays } from './format.js';

/* Order matters: it is the validated adjacency order for stacked marks. */
export const CATEGORIES = [
    { key: 'input_tokens', label: 'Input', hint: 'uncached prompt', color: 'var(--s-input)' },
    { key: 'output_tokens', label: 'Output', hint: 'incl. reasoning', color: 'var(--s-output)' },
    { key: 'cache_read_tokens', label: 'Cache read', hint: 'prompt cache hits', color: 'var(--s-cache-read)' },
    { key: 'cache_creation_tokens', label: 'Cache write', hint: 'prompt cache fills', color: 'var(--s-cache-write)' },
];

const FIELDS = [...CATEGORIES.map((category) => category.key), 'reasoning_tokens', 'total_tokens', 'cost_usd'];

export const RANGES = [
    { key: '7d', label: '7d', days: 7 },
    { key: '30d', label: '30d', days: 30 },
    { key: '90d', label: '90d', days: 90 },
    { key: '1y', label: '1y', days: 365 },
    { key: 'all', label: 'All', days: null },
];

export function emptyTotals() {
    return Object.fromEntries(FIELDS.map((field) => [field, 0]));
}

export function addInto(target, row) {
    for (const field of FIELDS) target[field] += Number(row[field]) || 0;
    return target;
}

export function sum(rows) {
    return rows.reduce((totals, row) => addInto(totals, row), emptyTotals());
}

/** Share of prompt tokens served from cache. */
export function cacheHitRate(totals) {
    const prompt = totals.input_tokens + totals.cache_read_tokens + totals.cache_creation_tokens;
    return prompt > 0 ? totals.cache_read_tokens / prompt : NaN;
}

export function categorySum(totals) {
    return CATEGORIES.reduce((total, category) => total + totals[category.key], 0);
}

export function rangeStart(range, today, firstDay) {
    const days = RANGES.find((item) => item.key === range)?.days;
    if (!days) return firstDay ?? today;
    return addDays(today, -(days - 1));
}

export function filterRows(rows, { start, end, agent }) {
    return rows.filter((row) => row.day >= start && row.day <= end && (agent === 'all' || row.agent === agent));
}

export function groupBy(rows, keyOf) {
    const groups = new Map();
    for (const row of rows) {
        const key = keyOf(row);
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(row);
    }
    return groups;
}

export function byAgent(rows) {
    return [...groupBy(rows, (row) => row.agent)]
        .map(([agent, items]) => ({ agent, totals: sum(items), models: new Set(items.map((row) => row.model)).size }))
        .sort((a, b) => b.totals.total_tokens - a.totals.total_tokens);
}

export function byModel(rows) {
    return [...groupBy(rows, (row) => `${row.agent}\u0000${row.model}`)].map(([key, items]) => {
        const days = items.map((row) => row.day).sort();
        const peak = items.reduce((best, row) => (row.total_tokens > (best?.total_tokens ?? -1) ? row : best), null);
        return {
            key,
            agent: items[0].agent,
            model: items[0].model,
            totals: sum(items),
            rows: items,
            activeDays: new Set(days).size,
            firstDay: days[0],
            lastDay: days[days.length - 1],
            peak,
        };
    });
}

/** Rows bucketed by day, week (Monday) or month, with every bucket from start to end present. */
export function buckets(rows, start, end) {
    const span = Math.round((Date.parse(end) - Date.parse(start)) / 86_400_000) + 1;
    const unit = span <= 92 ? 'day' : span <= 400 ? 'week' : 'month';

    const bucketOf = (day) => {
        if (unit === 'day') return day;
        if (unit === 'month') return `${day.slice(0, 7)}-01`;
        const date = new Date(`${day}T00:00:00Z`);
        const offset = (date.getUTCDay() + 6) % 7;
        return addDays(day, -offset);
    };

    const keys = [];
    for (let day = bucketOf(start); day <= end;) {
        keys.push(day);
        if (unit === 'day') day = addDays(day, 1);
        else if (unit === 'week') day = addDays(day, 7);
        else {
            const date = new Date(`${day}T00:00:00Z`);
            date.setUTCMonth(date.getUTCMonth() + 1);
            day = date.toISOString().slice(0, 10);
        }
    }

    const map = new Map(keys.map((key) => [key, emptyTotals()]));
    for (const row of rows) {
        const totals = map.get(bucketOf(row.day));
        if (totals) addInto(totals, row);
    }

    return { unit, items: keys.map((key) => ({ key, totals: map.get(key) })) };
}
