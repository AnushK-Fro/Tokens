export async function fetchSummary() {
    const response = await fetch('/api/summary', { cache: 'no-store', headers: { accept: 'application/json' } });
    if (!response.ok) throw new Error(`Summary request failed (${response.status})`);
    return response.json();
}
