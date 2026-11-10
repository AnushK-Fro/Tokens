import { byAgent } from '../aggregate.js';
import { h, mount } from '../dom.js';
import { compact, exact, money, percent } from '../format.js';
import { agentLabel, logo } from '../icons.js';

export function renderAgents(rows) {
    const agents = byAgent(rows);
    const total = agents.reduce((sum, agent) => sum + agent.totals.total_tokens, 0);

    if (!agents.length) {
        mount('#agents', h('p', { class: 'empty', text: 'No usage in this range.' }));
        return;
    }

    mount('#agents', agents.map(({ agent, totals, models }) => {
        const share = total ? totals.total_tokens / total : 0;
        return h('div', { class: 'agent-row' },
            logo(agent),
            h('p', null,
                h('span', { class: 'agent-name', text: agentLabel(agent) }),
                h('span', { class: 'agent-models', text: `${models} model${models === 1 ? '' : 's'} · ${percent(share)}` }),
            ),
            h('p', { class: 'agent-figures' },
                h('span', { title: exact(totals.total_tokens), text: compact(totals.total_tokens) }),
                h('span', { class: 'cost', text: money(totals.cost_usd) }),
            ),
            h('div', { class: 'share-track', role: 'presentation' }, h('div', { class: 'share-fill', style: { width: `${share * 100}%` } })),
        );
    }));
}
