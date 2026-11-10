const compactFormat = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });
const exactFormat = new Intl.NumberFormat('en');
const moneyFormat = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 });
const percentFormat = new Intl.NumberFormat('en', { style: 'percent', maximumFractionDigits: 1 });

export const compact = (value) => compactFormat.format(Number(value) || 0);
export const exact = (value) => exactFormat.format(Number(value) || 0);
export const percent = (value) => (Number.isFinite(value) ? percentFormat.format(value) : '—');

export function money(value) {
    const amount = Number(value) || 0;
    if (amount > 0 && amount < 0.01) return '<$0.01';
    return moneyFormat.format(amount);
}

/** Cost per million tokens. */
export function perMillion(cost, tokens) {
    return tokens > 0 ? money((cost / tokens) * 1e6) : '—';
}

/* Days travel as 'YYYY-MM-DD' strings and are only ever shifted in UTC, so a local clock
   change can't move them. */
export function parseDay(day) {
    const [year, month, date] = day.split('-').map(Number);
    return new Date(Date.UTC(year, month - 1, date));
}

export const dayKey = (date) => date.toISOString().slice(0, 10);

export function addDays(day, amount) {
    const date = parseDay(day);
    date.setUTCDate(date.getUTCDate() + amount);
    return dayKey(date);
}

export function daysBetween(from, to) {
    return Math.round((parseDay(to) - parseDay(from)) / 86_400_000);
}

const longDay = new Intl.DateTimeFormat(undefined, { weekday: 'short', year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' });
const shortDay = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' });
const monthYear = new Intl.DateTimeFormat(undefined, { month: 'short', year: 'numeric', timeZone: 'UTC' });
const mediumDate = new Intl.DateTimeFormat(undefined, { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' });

export const readableDay = (day) => longDay.format(parseDay(day));
export const shortDate = (day) => shortDay.format(parseDay(day));
export const monthLabel = (day) => monthYear.format(parseDay(day));
export const mediumDay = (day) => mediumDate.format(parseDay(day));

export function timeAgo(iso) {
    if (!iso) return 'never';
    const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
    if (seconds < 60) return 'just now';
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
    if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h ago`;
    return `${Math.floor(seconds / 86_400)}d ago`;
}
