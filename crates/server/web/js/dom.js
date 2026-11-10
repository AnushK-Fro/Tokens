const SVG_NS = 'http://www.w3.org/2000/svg';

function apply(node, props) {
    for (const [key, value] of Object.entries(props ?? {})) {
        if (value === undefined || value === null || value === false) continue;
        if (key === 'class') node.setAttribute('class', value);
        else if (key === 'text') node.textContent = value;
        else if (key === 'style' && typeof value === 'object') Object.assign(node.style, value);
        else if (key === 'dataset') Object.assign(node.dataset, value);
        else if (key.startsWith('on') && typeof value === 'function') node.addEventListener(key.slice(2), value);
        else node.setAttribute(key, value === true ? '' : value);
    }
}

function append(node, children) {
    for (const child of children.flat(Infinity)) {
        if (child === undefined || child === null || child === false) continue;
        node.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
}

/** `h('p', {class: 'x'}, 'text', childNode)` */
export function h(tag, props, ...children) {
    const node = document.createElement(tag);
    apply(node, props);
    append(node, children);
    return node;
}

export function s(tag, props, ...children) {
    const node = document.createElementNS(SVG_NS, tag);
    apply(node, props);
    append(node, children);
    return node;
}

export function mount(target, ...children) {
    const node = typeof target === 'string' ? document.querySelector(target) : target;
    node.replaceChildren();
    append(node, children);
    return node;
}
