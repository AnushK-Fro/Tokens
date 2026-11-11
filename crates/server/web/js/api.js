/** The browser's IANA zone, so days and "today" match the viewer's clock. */
function browserTimezone() {
    try {
        return Intl.DateTimeFormat().resolvedOptions().timeZone ?? '';
    } catch {
        return '';
    }
}

export async function fetchSummary() {
    const tz = browserTimezone();
    const url = tz ? `/api/summary?tz=${encodeURIComponent(tz)}` : '/api/summary';
    const response = await fetch(url, { cache: 'no-store', headers: { accept: 'application/json' } });
    if (!response.ok) throw new Error(`Summary request failed (${response.status})`);
    return response.json();
}
