import { h, mount } from './dom.js';

const tooltip = () => document.querySelector('#tooltip');

/**
 * Shows the shared tooltip next to `anchor` (an element).
 * `lines` are `[label, value, color?]`; a color adds a swatch beside the label.
 */
export function showTooltip(anchor, title, lines) {
    const node = tooltip();
    mount(node,
        h('h4', { text: title }),
        h('dl', null, lines.map(([label, value, color]) => [
            h('dt', null, color ? h('i', { class: 'swatch', style: { background: color } }) : null, label),
            h('dd', { text: value }),
        ])),
    );
    node.hidden = false;

    const box = anchor.getBoundingClientRect();
    const tip = node.getBoundingClientRect();
    const viewport = document.documentElement.clientWidth;
    const left = Math.min(Math.max(8, box.left + box.width / 2 - tip.width / 2), viewport - tip.width - 8);
    const above = box.top - tip.height - 10;
    node.style.left = `${left}px`;
    node.style.top = `${above < 8 ? box.bottom + 10 : above}px`;
}

export function hideTooltip() {
    tooltip().hidden = true;
}

/** Wires hover and keyboard focus on `[data-tip]` children of `container` to `describe(el)`. */
export function bindTooltips(container, describe) {
    const show = (event) => {
        const target = event.target.closest?.('[data-tip]');
        if (target && container.contains(target)) {
            const { title, lines } = describe(target);
            showTooltip(target, title, lines);
        } else {
            hideTooltip();
        }
    };
    container.addEventListener('pointerover', show);
    container.addEventListener('focusin', show);
    container.addEventListener('pointerleave', hideTooltip);
    container.addEventListener('focusout', hideTooltip);
}

window.addEventListener('scroll', hideTooltip, { passive: true });
