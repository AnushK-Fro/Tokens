/**
 * Model launches marked on the "Last 53 weeks" heatmap. Deliberately short: only launches that
 * visibly moved this dashboard's usage (the model was picked up within days and became a large
 * share of tokens). `label` is the on-chart text; `name` is shown on hover. Launches on the same
 * day share one line, each with its own label row. Keep in date order.
 */
export const LAUNCHES = [
    // Opus 5 in use from Jul 27; two-week volume rose ~80×.
    { day: '2026-07-24', label: 'Opus 5', name: 'Claude Opus 5' },
    // gpt-6-astra / gpt-daybreak in use from Sep 5.
    { day: '2026-09-03', label: 'GPT-6 Astra', name: 'GPT-6 Astra' },
    // Opus 5.5 in use the same day; gpt-6-sol from Sep 30, the largest model by tokens.
    { day: '2026-09-22', label: 'Opus 5.5', name: 'Claude Opus 5.5' },
    { day: '2026-09-22', label: 'GPT-6 Sol & Luna', name: 'GPT-6 Sol & Luna' },
];
