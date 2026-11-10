import { groupBy } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { mediumDay, money } from '../format.js';
import { logo } from '../icons.js';

const PAGE = 8;

/** Billing accounts as they appear in `payments[].provider`. */
const PROVIDERS = {
    chatgpt: { label: 'ChatGPT', logo: 'openai' },
    claude: { label: 'Claude', logo: 'claude' },
    cursor: { label: 'Cursor', logo: 'cursor' },
    kimi: { label: 'Kimi', logo: 'kimi' },
    zai: { label: 'Z.ai', logo: 'zai' },
    opencode: { label: 'OpenCode Go', logo: 'opencode' },
    openrouter: { label: 'OpenRouter', logo: 'openrouter' },
    'openai-api': { label: 'OpenAI API', logo: 'openai' },
};

const providerLabel = (provider) => PROVIDERS[provider]?.label ?? provider;
const providerLogo = (provider) => logo(PROVIDERS[provider]?.logo ?? provider);

/**
 * Real payments next to what the same range's usage would have cost at API prices.
 * `payments` arrive newest first. `start` is null for all time: payments then go back further
 * than any stored usage. `ui.shown` persists the "show more" depth across refreshes.
 */
export function renderSpending({ payments, start, end, agent, apiCost, rangeLabel, ui, onChange }) {
    document.querySelector('#spending-sub').textContent = `What was actually paid, ${rangeLabel}`;

    const scoped = payments.filter((payment) => payment.amount_usd > 0
        && (start === null || payment.paid_on >= start) && payment.paid_on <= end
        && (agent === 'all' || payment.agent === agent));

    if (!scoped.length) {
        mount('#spending', h('p', { class: 'empty', text: 'No payments in this range.' }));
        return;
    }

    const paid = scoped.reduce((total, payment) => total + payment.amount_usd, 0);
    const providers = [...groupBy(scoped, (payment) => payment.provider)]
        .map(([provider, items]) => ({
            provider,
            items,
            total: items.reduce((total, payment) => total + payment.amount_usd, 0),
        }))
        .sort((a, b) => b.total - a.total);
    const shown = scoped.slice(0, ui.shown ?? PAGE);

    mount('#spending',
        h('div', { class: 'spend' },
            h('div', null,
                providers.map(({ provider, items, total }) => h('div', { class: 'agent-row' },
                    providerLogo(provider),
                    h('p', null,
                        h('span', { class: 'agent-name', text: providerLabel(provider) }),
                        h('span', {
                            class: 'agent-models',
                            text: [
                                items[0].product === providerLabel(provider) ? null : items[0].product,
                                `${items.length} charge${items.length === 1 ? '' : 's'}`,
                            ].filter(Boolean).join(' · '),
                        }),
                    ),
                    h('p', { class: 'agent-figures' }, h('span', { class: 'cost', text: money(total) })),
                    h('div', { class: 'share-track', role: 'presentation' }, h('div', { class: 'share-fill', style: { width: `${(total / paid) * 100}%` } })),
                )),
                h('div', { class: 'mix-notes' },
                    note('Paid', money(paid)),
                    note('Usage at API prices', money(apiCost), 'ccusage estimate for the same range and agents'),
                    note('API value per $1', paid > 0 && apiCost > 0 ? `$${(apiCost / paid).toFixed(1)}` : '—'),
                ),
            ),
            h('div', null,
                h('ol', { class: 'payments' }, shown.map((payment) => h('li', { class: 'payment' },
                    h('span', { class: 'payment-day', text: mediumDay(payment.paid_on) }),
                    h('span', { class: 'payment-product', title: providerLabel(payment.provider) },
                        providerLogo(payment.provider),
                        h('span', { text: payment.product }),
                        payment.note ? h('small', { text: payment.note }) : null,
                    ),
                    h('span', { class: 'payment-amount', text: money(payment.amount_usd) }),
                ))),
                shown.length < scoped.length
                    ? h('button', {
                        class: 'more',
                        type: 'button',
                        text: `Show ${Math.min(PAGE, scoped.length - shown.length)} more`,
                        onclick: () => {
                            ui.shown = shown.length + PAGE;
                            onChange();
                        },
                    })
                    : null,
            ),
        ),
    );
}

function note(label, value, title) {
    return h('div', { class: 'mix-note', title }, h('span', { class: 'label', text: label }), h('strong', { text: value }));
}
